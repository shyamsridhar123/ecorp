//! Read-only authority revalidation for the existing durable VerifyRun path.
//! Artifact hydration is optional; current recovery authority is not.

use super::*;

fn verification_command_identity_matches(command: &PendingRunnerCommand) -> bool {
    command.command_kind == "factory_verification_recovery"
        && matches!(
            command.payload.get("mode").and_then(Value::as_str),
            Some("verifier_only" | "checkpoint_verification")
        )
        && [("corp_id", command.corp_id), ("run_id", command.run_id)]
            .into_iter()
            .all(|(field, expected)| {
                command
                    .payload
                    .get(field)
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    == Some(expected)
            })
}

impl PgStore {
    /// Recheck the exact pending verifier command against current native authority.
    ///
    /// Call immediately before VerifyRun dispatch, including after optional object
    /// storage I/O. False grants no dispatch authority; the caller owns the native
    /// command/run failure path. This read neither settles commands nor mutates runs.
    /// Artifact signatures and bytes still require the existing transfer validation.
    pub async fn verification_recovery_dispatch_authorized(
        &self,
        command: &PendingRunnerCommand,
    ) -> Result<bool> {
        if !verification_command_identity_matches(command) {
            return Ok(false);
        }
        Ok(sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM runner_commands stored
                JOIN runs run
                  ON run.id=stored.run_id AND run.corp_id=stored.corp_id
                 AND run.runner_id=stored.runner_id
                JOIN factory_verification_recoveries recovery
                  ON recovery.replacement_run_id=run.id AND recovery.corp_id=run.corp_id
                 AND recovery.task_id=run.task_id
                JOIN tasks task
                  ON task.id=run.task_id AND task.corp_id=run.corp_id
                 AND task.mission_id=recovery.mission_id
                JOIN missions mission
                  ON mission.id=task.mission_id AND mission.corp_id=run.corp_id
                JOIN rooms room
                  ON room.id=mission.room_id AND room.corp_id=run.corp_id
                JOIN factory_work_items item
                  ON item.id=recovery.factory_work_item_id AND item.corp_id=run.corp_id
                 AND item.mission_id=mission.id
                JOIN actors author
                  ON author.id=recovery.authorized_by AND author.corp_id=run.corp_id
                 AND author.kind='human' AND author.role IN ('owner','admin','manager')
                JOIN room_memberships membership
                  ON membership.room_id=room.id AND membership.actor_id=author.id
                JOIN runs source
                  ON source.id=recovery.source_run_id AND source.corp_id=run.corp_id
                 AND source.id=run.resumed_from_run_id AND source.task_id=run.task_id
                 AND source.agent_id=run.agent_id AND source.runner_id=run.runner_id
                 AND source.workspace_run_id=run.workspace_run_id
                 AND source.source_repository IS NOT DISTINCT FROM run.source_repository
                 AND source.source_base_ref IS NOT DISTINCT FROM run.source_base_ref
                 AND source.source_base_commit IS NOT DISTINCT FROM run.source_base_commit
                WHERE stored.id=$1 AND stored.corp_id=$2
                  AND stored.run_id=$3 AND stored.runner_id=$4
                  AND stored.command_kind='factory_verification_recovery'
                  AND stored.status='pending' AND stored.payload=$5::jsonb
                  AND run.execution_mode='verification_only'
                  AND run.provider_session_id IS NULL AND run.model IS NULL
                  AND run.reasoning_effort IS NULL
                  AND run.status IN ('provisioning','starting','running','verifying',
                                     'waiting_for_input','waiting_for_approval')
                  AND run.workspace_disposition IS DISTINCT FROM 'quarantined'
                  AND recovery.mode IN ('verifier_only','checkpoint_verification')
                  AND recovery.status IN ('authorized','running')
                  AND source.status IN ('failed','cancelled','lost')
                  AND source.workspace_disposition='preserved'
                  AND NULLIF(source.workspace_path,'') IS NOT NULL
                  AND stored.payload->>'mode'=recovery.mode
                  AND stored.payload->>'corp_id'=run.corp_id::text
                  AND stored.payload->>'run_id'=run.id::text
                  AND stored.payload->>'task_id'=task.id::text
                  AND stored.payload->>'mission_id'=mission.id::text
                  AND stored.payload->>'room_id'=mission.room_id::text
                  AND stored.payload->>'agent_id'=run.agent_id::text
                  AND stored.payload->>'assignment_token'=run.assignment_token::text
                  AND stored.payload->>'workspace_run_id'=run.workspace_run_id::text
                  AND stored.payload->>'source_run_id'=source.id::text
                  AND stored.payload->>'source_repository'
                      IS NOT DISTINCT FROM run.source_repository
                  AND stored.payload->>'source_base_ref'
                      IS NOT DISTINCT FROM run.source_base_ref
                  AND stored.payload->>'source_base_commit'
                      IS NOT DISTINCT FROM run.source_base_commit
                  AND task.contract->>'source_repository'
                      IS NOT DISTINCT FROM run.source_repository
                  AND task.contract->>'source_base_ref'
                      IS NOT DISTINCT FROM run.source_base_ref
                  AND task.contract->>'source_base_commit'
                      IS NOT DISTINCT FROM run.source_base_commit
                  AND stored.payload->>'workspace_base_commit'=source.workspace_base_commit
                  AND stored.payload->>'expected_workspace_fingerprint'=source.workspace_fingerprint
                  AND stored.payload->>'expected_workspace_fingerprint'
                      =recovery.request->>'expected_workspace_fingerprint'
                  AND stored.payload->>'expected_head_commit'
                      IS NOT DISTINCT FROM recovery.request->>'expected_head_commit'
                  AND stored.payload->'verification_policy'=recovery.replacement_verification_policy
                  AND recovery.replacement_verification_policy=task.verification_policy
                  AND stored.payload->'write_scope'=task.contract->'write_scope'
                  AND COALESCE(stored.payload->'deliverable','null'::jsonb)
                      =COALESCE(task.contract->'deliverable','null'::jsonb)
                  AND COALESCE(stored.payload->'secret_refs','[]'::jsonb)='[]'::jsonb
                  AND (
                      recovery.mode <> 'checkpoint_verification'
                      OR NOT EXISTS (
                          SELECT 1 FROM runs lineage
                          JOIN events stop_event
                            ON stop_event.corp_id=lineage.corp_id AND stop_event.aggregate_id=lineage.id
                           AND stop_event.aggregate_type='run' AND stop_event.type='run.stop_requested'
                          WHERE lineage.corp_id=run.corp_id
                            AND lineage.workspace_run_id=run.workspace_run_id
                      )
                  )
            )
            "#,
        )
        .bind(command.id)
        .bind(command.corp_id)
        .bind(command.run_id)
        .bind(&command.runner_id)
        .bind(&command.payload)
        .fetch_one(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> PendingRunnerCommand {
        let corp_id = Uuid::from_u128(1);
        let run_id = Uuid::from_u128(2);
        PendingRunnerCommand {
            id: Uuid::from_u128(3),
            corp_id,
            run_id,
            runner_id: "verification-dispatch-fixture".to_owned(),
            command_kind: "factory_verification_recovery".to_owned(),
            payload: json!({
                "mode":"checkpoint_verification",
                "corp_id":corp_id,
                "run_id":run_id,
                "provider_artifact":null,
            }),
        }
    }

    #[test]
    fn verification_dispatch_identity_does_not_require_an_artifact() {
        for mode in ["verifier_only", "checkpoint_verification"] {
            let mut command = command();
            command.payload["mode"] = json!(mode);
            assert!(verification_command_identity_matches(&command));
            command.payload["provider_artifact"] = json!({"fixture":"metadata only"});
            assert!(verification_command_identity_matches(&command));
        }
    }

    #[test]
    fn verification_dispatch_identity_rejects_wrong_or_malformed_scope() {
        for field in ["corp_id", "run_id"] {
            for value in [Value::Null, json!(42), json!("invalid"), json!(Uuid::nil())] {
                let mut command = command();
                command.payload[field] = value;
                assert!(!verification_command_identity_matches(&command));
            }
            let mut command = command();
            command.payload.as_object_mut().unwrap().remove(field);
            assert!(!verification_command_identity_matches(&command));
        }
    }

    #[test]
    fn verification_dispatch_identity_rejects_other_command_modes_without_mutation() {
        for mode in ["source_correction", "provider", "unknown"] {
            let mut command = command();
            command.payload["mode"] = json!(mode);
            let before = command.payload.clone();
            assert!(!verification_command_identity_matches(&command));
            assert_eq!(command.payload, before);
        }
        let mut command = command();
        command.command_kind = "circuit_breaker".to_owned();
        assert!(!verification_command_identity_matches(&command));
    }
}
