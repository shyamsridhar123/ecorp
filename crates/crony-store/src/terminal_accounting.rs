//! Termination is an assignment-scoped fact, not authority to advance a run.

use super::*;

pub(super) async fn validate_termination_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    execution_mode: &str,
    payload: &Value,
) -> Result<()> {
    // The enclosing transaction has already fenced the exact run/agent/runner/
    // assignment. Never infer an old assignment from mutable current agent
    // configuration. Legacy unpinned tasks need unambiguous native start evidence.
    let row = sqlx::query(
        "SELECT task.required_adapter,
                ARRAY(SELECT DISTINCT event.payload->>'adapter'
                      FROM events event
                      WHERE event.corp_id=$2 AND event.aggregate_id=$3
                        AND event.aggregate_type='run' AND event.type='run.started') AS started_adapters
         FROM tasks task WHERE task.id=$1 AND task.corp_id=$2",
    )
    .bind(task_id)
    .bind(corp_id)
    .bind(run_id)
    .fetch_one(&mut **tx)
    .await?;
    let declared: Option<String> = row.get("required_adapter");
    let started: Vec<Option<String>> = row.get("started_adapters");
    let adapter = match (declared, started.as_slice()) {
        (Some(adapter), []) => adapter,
        (Some(adapter), [Some(started)]) if adapter == *started => adapter,
        (None, [Some(started)]) => started.clone(),
        _ => {
            return Err(anyhow!(
                "provider-termination assignment adapter is unproven"
            ));
        }
    };
    validate_termination(payload, &adapter, execution_mode)
}

fn validate_termination(payload: &Value, adapter: &str, execution_mode: &str) -> Result<()> {
    let valid = execution_mode == "provider"
        && payload.as_object().is_some_and(|object| {
            object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "adapter" | "outcome" | "provider_process_alive" | "message"
                )
            })
        })
        && !adapter.is_empty()
        && payload.get("adapter").and_then(Value::as_str) == Some(adapter)
        && payload.get("provider_process_alive") == Some(&Value::Bool(false))
        && matches!(
            payload.get("outcome").and_then(Value::as_str),
            Some("completed" | "cancelled" | "failed" | "runtime_error")
        )
        && payload.get("message").is_none_or(|message| {
            message.as_str().is_some_and(|message| {
                !message.trim().is_empty()
                    && message.len() <= 1_024
                    && !message.chars().any(char::is_control)
            })
        });
    if !valid {
        return Err(anyhow!("invalid native provider-termination evidence"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORP: Uuid = Uuid::from_u128(1);
    const MISSION: Uuid = Uuid::from_u128(2);
    const TASK: Uuid = Uuid::from_u128(3);
    const RUN: Uuid = Uuid::from_u128(4);
    const AGENT: Uuid = Uuid::from_u128(5);
    const ROOM: Uuid = Uuid::from_u128(6);
    const FACTORY: Uuid = Uuid::from_u128(7);
    const TOKEN: Uuid = Uuid::from_u128(8);

    fn payload() -> Value {
        json!({
            "adapter":"codex", "outcome":"cancelled", "provider_process_alive":false,
            "message":"Provider session stopped; no idle agent process remains."
        })
    }

    fn input(event_type: &str, payload: Value) -> RunnerEventInput {
        RunnerEventInput {
            event_id: Uuid::new_v4(),
            runner_id: "issue193-runner".to_owned(),
            corp_id: CORP,
            connection_epoch: Uuid::from_u128(20),
            run_id: RUN,
            agent_id: AGENT,
            assignment_token: TOKEN,
            event_type: event_type.to_owned(),
            payload,
        }
    }

    fn invalid_payloads() -> Vec<Value> {
        let mut values = vec![Value::Null, json!([])];
        for (key, value) in [
            ("adapter", json!("other-adapter")),
            ("adapter", Value::Null),
            ("outcome", json!("verified")),
            ("outcome", Value::Null),
            ("provider_process_alive", json!(true)),
            ("provider_process_alive", json!("false")),
            ("message", json!(42)),
            ("message", json!("")),
            ("message", json!("x".repeat(1_025))),
            ("message", json!("untrusted\nmessage")),
            ("credential", json!("must not be retained")),
        ] {
            let mut candidate = payload();
            candidate[key] = value;
            values.push(candidate);
        }
        for key in ["adapter", "outcome", "provider_process_alive"] {
            let mut candidate = payload();
            candidate.as_object_mut().unwrap().remove(key);
            values.push(candidate);
        }
        values
    }

    #[test]
    fn issue193_native_termination_shape_is_explicit_and_bounded() {
        for outcome in ["completed", "cancelled", "failed", "runtime_error"] {
            let mut candidate = payload();
            candidate["outcome"] = json!(outcome);
            assert!(validate_termination(&candidate, "codex", "provider").is_ok());
        }
        let mut without_message = payload();
        without_message.as_object_mut().unwrap().remove("message");
        assert!(validate_termination(&without_message, "codex", "provider").is_ok());
        for candidate in invalid_payloads() {
            let error = validate_termination(&candidate, "codex", "provider").unwrap_err();
            assert_eq!(
                error.to_string(),
                "invalid native provider-termination evidence"
            );
        }
    }

    #[test]
    fn issue193_verifier_only_work_cannot_claim_provider_termination() {
        assert!(validate_termination(&payload(), "codex", "verification_only").is_err());
        assert!(validate_termination(&payload(), "", "provider").is_err());
    }

    async fn fixture(pool: PgPool) -> PgStore {
        sqlx::raw_sql(
            r#"
            INSERT INTO corps(id,slug,name) VALUES
                ('00000000-0000-0000-0000-000000000001','issue193','Issue193 fixture');
            INSERT INTO actors(id,corp_id,name,kind,role) VALUES
                ('00000000-0000-0000-0000-000000000009','00000000-0000-0000-0000-000000000001','Owner','human','owner'),
                ('00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000000001','Worker','agent','worker');
            INSERT INTO rooms(id,corp_id,name,purpose) VALUES
                ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000001','Fixture','Late terminal accounting');
            INSERT INTO room_memberships(room_id,actor_id) VALUES
                ('00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000009');
            INSERT INTO agents(id,corp_id,actor_id,name,role,adapter,status,current_run_id,accent) VALUES
                ('00000000-0000-0000-0000-000000000005','00000000-0000-0000-0000-000000000001',
                 '00000000-0000-0000-0000-00000000000a','Worker','worker','codex','running',
                 '00000000-0000-0000-0000-000000000004','#123456');
            INSERT INTO missions(id,corp_id,room_id,requested_by,title,status,budget_tokens,
                                 original_budget_tokens,original_budget_cost_microusd) VALUES
                ('00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000001',
                 '00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000009',
                 'Retain terminal evidence','running',1000000,1000000,5000000);
            INSERT INTO factory_work_items
                (id,corp_id,source_kind,source_project_owner,source_project_number,
                 source_project_item_id,source_repository_owner,source_repository_name,
                 source_issue_number,source_issue_node_id,source_issue_url,source_title,
                 source_revision,state,claim_owner_id,claim_token,lease_expires_at,mission_id,policy)
            VALUES ('00000000-0000-0000-0000-000000000007','00000000-0000-0000-0000-000000000001',
                    'github_project_issue','fixture',193,'item-193','fixture','source',193,'issue-193',
                    'https://github.com/fixture/source/issues/193','Late terminal evidence','revision-1',
                    'running','00000000-0000-0000-0000-000000000009',
                    '00000000-0000-0000-0000-000000000010',now()+interval '1 hour',
                    '00000000-0000-0000-0000-000000000002','{"fixture":true}');
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        let contract = json!({
            "objective":"retain base.txt","expected_output":"base.txt",
            "source_repository":"fixture/source","source_base_ref":"main",
            "source_base_commit":"a".repeat(40),"acceptance_tests":["base.txt exists"],
            "allowed_tools":["filesystem"],"prohibited_actions":["leave the worktree"],
            "references":[],"write_scope":["base.txt"],"budget_tokens":100000,
            "budget_cost_microusd":1000000,"deadline_at":null,"escalation":"ask owner",
            "secret_refs":[],"model":null,"reasoning_effort":null,"deliverable":null
        });
        sqlx::query(
            "INSERT INTO tasks(id,corp_id,mission_id,title,objective,status,assigned_agent_id,
                               plan_key,contract,verification_policy,attempt_count,max_attempts,required_adapter)
             VALUES($1,$2,$3,'Root','retain source','running',$4,'root',$5,$6,2,2,'codex')",
        )
        .bind(TASK).bind(CORP).bind(MISSION).bind(AGENT).bind(contract)
        .bind(json!({"checks":[{"type":"file","path":"base.txt","min_bytes":1}],"manual_gate":null}))
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,
                              workspace_run_id,provider_session_id,workspace_disposition,
                              workspace_path,workspace_branch,workspace_base_ref,workspace_base_commit,
                              workspace_fingerprint,source_repository,source_base_ref,source_base_commit,
                              input_tokens,output_tokens,cost_microusd,breaker_stage)
             VALUES($1,$2,$3,$4,'issue193-runner',$5,'running',$1,'fixture-session','preserved',
                    'fixture-worktree','crony/fixture','main',$6,$7,'fixture/source','main',$6,6000,0,0,'stop')",
        )
        .bind(RUN).bind(CORP).bind(TASK).bind(AGENT).bind(TOKEN)
        .bind("a".repeat(40)).bind("b".repeat(64))
        .execute(&pool).await.unwrap();
        PgStore { pool }
    }

    async fn failed_fixture(pool: PgPool) -> PgStore {
        let store = fixture(pool).await;
        store
            .apply_runner_event(input(
                "run.failed",
                json!({"error":"artifact upload rejected after a hard budget stop"}),
            ))
            .await
            .unwrap();
        store
    }

    async fn state(store: &PgStore) -> Value {
        sqlx::query_scalar(
            "SELECT jsonb_build_object(
                'runs',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM runs x),
                'tasks',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM tasks x),
                'missions',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM missions x),
                'agents',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM agents x),
                'factory',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM factory_work_items x),
                'events',(SELECT COALESCE(jsonb_agg(to_jsonb(x) ORDER BY seq),'[]') FROM events x),
                'commands',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM runner_commands x),
                'evidence',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM verification_evidence x),
                'reviews',(SELECT jsonb_agg(to_jsonb(x) ORDER BY run_id) FROM verification_requests x))",
        ).fetch_one(&store.pool).await.unwrap()
    }

    async fn assert_append_only(store: &PgStore) {
        let before = state(store).await;
        let request = input("run.session_terminated", payload());
        let outcome = store.apply_runner_event(request.clone()).await.unwrap();
        let event = outcome.event.unwrap();
        assert_eq!(event.id, request.event_id);
        assert_eq!(event.room_id, Some(ROOM));
        assert_eq!(event.correlation_id, Some(MISSION));
        assert_eq!(event.aggregate_id, RUN);
        assert_eq!(event.event_type, "run.session_terminated");
        assert_eq!(event.payload, payload());
        assert!(outcome.related_events.is_empty());
        let after = state(store).await;
        for key in before
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| key.as_str() != "events")
        {
            assert_eq!(before[key], after[key], "{key}");
        }
        let prior = before["events"].as_array().unwrap();
        let current = after["events"].as_array().unwrap();
        assert_eq!(current.len(), prior.len() + 1);
        assert_eq!(&current[..prior.len()], prior);
        let replay = store.apply_runner_event(request).await.unwrap();
        assert!(replay.event.is_none());
        assert!(replay.related_events.is_empty());
        assert_eq!(state(store).await, after);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_native_failure_accepts_only_append_only_termination(pool: PgPool) {
        let store = failed_fixture(pool).await;
        assert_eq!(state(&store).await["runs"][0]["status"], "failed");
        assert_append_only(&store).await;
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_protected_terminal_states_and_quarantine_never_change(pool: PgPool) {
        let store = failed_fixture(pool).await;
        for status in ["completed", "cancelled", "lost"] {
            sqlx::query(
                "UPDATE runs SET status=$1,workspace_disposition='quarantined' WHERE id=$2",
            )
            .bind(status)
            .bind(RUN)
            .execute(&store.pool)
            .await
            .unwrap();
            sqlx::query("UPDATE factory_work_items SET state='published' WHERE id=$1")
                .bind(FACTORY)
                .execute(&store.pool)
                .await
                .unwrap();
            assert_append_only(&store).await;
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_other_assignment_pointer_and_all_active_states_remain_untouched(
        pool: PgPool,
    ) {
        let store = failed_fixture(pool).await;
        let other = Uuid::from_u128(30);
        sqlx::query(
            "INSERT INTO runs(id,corp_id,task_id,agent_id,runner_id,assignment_token,status,workspace_run_id)
             VALUES($1,$2,$3,$4,'issue193-runner',$5,'running',$1)",
        ).bind(other).bind(CORP).bind(TASK).bind(AGENT).bind(Uuid::new_v4())
            .execute(&store.pool).await.unwrap();
        sqlx::query("UPDATE agents SET status='running',current_run_id=$1,station='implementation' WHERE id=$2")
            .bind(other).bind(AGENT).execute(&store.pool).await.unwrap();
        for status in [
            "provisioning",
            "starting",
            "running",
            "waiting_for_input",
            "waiting_for_approval",
            "verifying",
        ] {
            sqlx::query("UPDATE runs SET status=$1 WHERE id=$2")
                .bind(status)
                .bind(other)
                .execute(&store.pool)
                .await
                .unwrap();
            assert_append_only(&store).await;
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_foreign_assignment_dimensions_never_append(pool: PgPool) {
        let store = failed_fixture(pool).await;
        for field in ["corp", "run", "agent", "runner", "token"] {
            let before = state(&store).await;
            let mut request = input("run.session_terminated", payload());
            match field {
                "corp" => request.corp_id = Uuid::new_v4(),
                "run" => request.run_id = Uuid::new_v4(),
                "agent" => request.agent_id = Uuid::new_v4(),
                "runner" => request.runner_id = "another-runner".to_owned(),
                "token" => request.assignment_token = Uuid::new_v4(),
                _ => unreachable!(),
            }
            assert!(store.apply_runner_event(request).await.is_err(), "{field}");
            assert_eq!(state(&store).await, before);
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_invalid_terminal_payloads_never_append(pool: PgPool) {
        let store = failed_fixture(pool).await;
        for candidate in invalid_payloads() {
            let before = state(&store).await;
            assert!(
                store
                    .apply_runner_event(input("run.session_terminated", candidate))
                    .await
                    .is_err()
            );
            assert_eq!(state(&store).await, before);
        }
        sqlx::query("UPDATE runs SET execution_mode='verification_only' WHERE id=$1")
            .bind(RUN)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(
            store
                .apply_runner_event(input("run.session_terminated", payload()))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_terminal_progress_remains_rejected(pool: PgPool) {
        let store = failed_fixture(pool).await;
        for event in [
            "run.started",
            "run.status",
            "run.artifact_upload",
            "run.deliverable_upload",
            "run.verification_started",
            "run.verification_evidence",
            "run.verification_passed",
            "run.verification_waiting",
            "run.completed",
            "run.usage",
            "run.output",
            "run.cancelled",
        ] {
            let before = state(&store).await;
            assert!(
                store
                    .apply_runner_event(input(event, json!({})))
                    .await
                    .is_err(),
                "{event}"
            );
            assert_eq!(state(&store).await, before);
        }
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_active_termination_validates_without_changing_projection(pool: PgPool) {
        let store = fixture(pool).await;
        assert_append_only(&store).await;
    }

    async fn start_evidence(store: &PgStore, adapter: &str) {
        let mut tx = store.pool.begin().await.unwrap();
        append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(ROOM),
                correlation_id: Some(MISSION),
                ..NewEvent::new(
                    CORP,
                    None,
                    "run.started",
                    "run",
                    RUN,
                    format!("issue193-start:{}", Uuid::new_v4()),
                    json!({"adapter":adapter,"task_id":TASK}),
                )
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_legacy_start_evidence_binds_adapter_not_current_agent_config(pool: PgPool) {
        let store = fixture(pool).await;
        start_evidence(&store, "codex").await;
        store
            .apply_runner_event(input(
                "run.failed",
                json!({"error":"native late upload rejection"}),
            ))
            .await
            .unwrap();
        sqlx::query("UPDATE tasks SET required_adapter=NULL WHERE id=$1")
            .bind(TASK)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE agents SET adapter='github-copilot' WHERE id=$1")
            .bind(AGENT)
            .execute(&store.pool)
            .await
            .unwrap();
        assert_append_only(&store).await;
        let before = state(&store).await;
        let mut wrong = payload();
        wrong["adapter"] = json!("github-copilot");
        assert!(
            store
                .apply_runner_event(input("run.session_terminated", wrong))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_missing_ambiguous_or_conflicting_adapter_authority_is_rejected(pool: PgPool) {
        let store = failed_fixture(pool).await;
        sqlx::query("UPDATE tasks SET required_adapter=NULL WHERE id=$1")
            .bind(TASK)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(
            store
                .apply_runner_event(input("run.session_terminated", payload()))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
        start_evidence(&store, "codex").await;
        start_evidence(&store, "github-copilot").await;
        let before = state(&store).await;
        assert!(
            store
                .apply_runner_event(input("run.session_terminated", payload()))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
        sqlx::query("UPDATE tasks SET required_adapter='codex' WHERE id=$1")
            .bind(TASK)
            .execute(&store.pool)
            .await
            .unwrap();
        let before = state(&store).await;
        assert!(
            store
                .apply_runner_event(input("run.session_terminated", payload()))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
    }

    #[sqlx::test(migrations = "../../db/migrations")]
    #[ignore = "requires the explicitly owned issue193 SQLx maintenance fixture"]
    async fn issue193_event_failure_rolls_back_everything(pool: PgPool) {
        let store = failed_fixture(pool).await;
        sqlx::raw_sql(
            "CREATE FUNCTION reject_issue193_event() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN IF NEW.type='run.session_terminated' THEN RAISE EXCEPTION 'issue193 injected event failure'; END IF;
             RETURN NEW; END; $$;
             CREATE TRIGGER issue193_event BEFORE INSERT ON events
             FOR EACH ROW EXECUTE FUNCTION reject_issue193_event();",
        ).execute(&store.pool).await.unwrap();
        let before = state(&store).await;
        assert!(
            store
                .apply_runner_event(input("run.session_terminated", payload()))
                .await
                .is_err()
        );
        assert_eq!(state(&store).await, before);
    }
}
