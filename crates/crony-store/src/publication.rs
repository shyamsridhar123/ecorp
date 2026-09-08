use super::*;

const PUBLICATION_SELECT: &str = r#"
    SELECT publication.id, publication.corp_id, publication.factory_work_item_id,
           publication.mission_id, publication.source_deliverable_id,
           publication.artifact_id, publication.task_id, publication.run_id,
           publication.source_issue_number, publication.source_issue_url,
           publication.target_repository, publication.base_ref, publication.branch,
           publication.commit_sha, publication.title, publication.body,
           publication.actor_id, publication.authorization_id,
           publication.authorization_snapshot, publication.effect_key,
           publication.idempotency_key, publication.state, publication.version,
           publication.attempt_count, publication.publisher_id,
           publication.publisher_token, publication.publisher_lease_expires_at,
           publication.failure_detail, publication.branch_pushed_at,
           publication.pull_request_number, publication.pull_request_node_id,
           publication.pull_request_url, publication.pull_request_state,
           publication.pull_request_draft, publication.pull_request_base_ref,
           publication.pull_request_head_sha,
           publication.pull_request_head_repository_owner,
           publication.pull_request_is_cross_repository, publication.project_owner,
           publication.project_number, publication.project_item_id,
           publication.project_status_before, publication.project_status_after,
           publication.project_status_updated_at, publication.auto_merge_enabled,
           publication.merge_authorized, publication.deployment_authorized,
           publication.provenance, publication.created_at, publication.updated_at
    FROM pull_request_publications publication
"#;

#[derive(Debug, Clone)]
struct PublicationOperation {
    publication_id: Uuid,
    actor_id: Uuid,
    operation: String,
    resulting_version: i64,
    publisher_token: Option<Uuid>,
    request: Value,
}

struct NewPublicationOperation<'a> {
    corp_id: Uuid,
    idempotency_key: &'a str,
    publication_id: Uuid,
    actor_id: Uuid,
    operation: &'a str,
    resulting_version: i64,
    publisher_token: Option<Uuid>,
    request: &'a Value,
}

#[derive(Debug)]
struct PublicationPrerequisites {
    work_item: FactoryWorkItem,
    effective_source_revision: String,
    source_recovery_id: Option<Uuid>,
    mission_id: Uuid,
    room_id: Uuid,
    artifact_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    commit_sha: String,
    deliverable_sha256: String,
    verification_sha256: String,
    base_commit: String,
    source_branch: String,
    project_status_before: String,
    review_status: String,
    task_ids: Vec<Uuid>,
    run_ids: Vec<Uuid>,
    evidence_ids: Vec<Uuid>,
    checkpoint: Option<checkpoint_publication::CheckpointPublication>,
}

struct PublicationPrerequisiteRequest<'a> {
    corp_id: Uuid,
    work_item_id: Uuid,
    source_deliverable_id: Uuid,
    target_repository: &'a str,
    base_ref: &'a str,
    branch: &'a str,
    body: &'a str,
}

struct ActivePublicationControl<'a> {
    actor_id: Uuid,
    publisher_id: &'a str,
    presented_token: Uuid,
    expected_version: i64,
    now: chrono::DateTime<Utc>,
}

impl PgStore {
    pub async fn factory_publication_context(
        &self,
        corp_id: Uuid,
        viewer_actor_id: Uuid,
        work_item_id: Uuid,
    ) -> Result<Option<FactoryPublicationContext>> {
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, corp_id, viewer_actor_id).await?;
        let work_item = sqlx::query(
            r#"
            SELECT id, corp_id, source_kind, source_project_owner, source_project_number,
                   source_project_item_id, source_repository_owner, source_repository_name,
                   source_issue_number, source_issue_node_id, source_issue_url, source_title,
                   source_revision, state, version, claim_owner_id, lease_expires_at, policy,
                   mission_id, failure_detail, created_at, updated_at
            FROM factory_work_items
            WHERE id = $1
              AND corp_id = $2
              AND EXISTS (
                  SELECT 1
                  FROM missions mission
                  JOIN room_memberships membership
                    ON membership.room_id = mission.room_id
                  WHERE mission.id = factory_work_items.mission_id
                    AND mission.corp_id = factory_work_items.corp_id
                    AND membership.actor_id = $3
              )
              AND EXISTS (
                  SELECT 1
                  FROM actors viewer
                  WHERE viewer.id = $3
                    AND viewer.corp_id = factory_work_items.corp_id
                    AND viewer.kind = 'human'
                    AND viewer.role IN ('owner', 'admin', 'manager', 'member')
              )
            "#,
        )
        .bind(work_item_id)
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(map_factory_work_item)
        .transpose()?;
        let Some(work_item) = work_item else {
            tx.commit().await?;
            return Ok(None);
        };

        let publication =
            publication_for_work_item_viewer_tx(&mut tx, corp_id, viewer_actor_id, work_item_id)
                .await?
                .map(|(publication, _)| publication);

        let source_deliverables = if let Some(mission_id) = work_item.mission_id {
            sqlx::query(
                r#"
                SELECT deliverable.id, deliverable.corp_id, deliverable.task_id,
                       deliverable.run_id, deliverable.artifact_id, deliverable.form,
                       deliverable.file_name, artifact.uri, artifact.sha256,
                       artifact.media_type, artifact.bytes, artifact.provenance_signature,
                       deliverable.verification_sha256, deliverable.base_commit,
                       deliverable.head_commit, deliverable.branch,
                       deliverable.integration_state, artifact.retention_until,
                       deliverable.created_at
                FROM source_deliverables deliverable
                JOIN artifacts artifact
                  ON artifact.id = deliverable.artifact_id AND artifact.status = 'ready'
                JOIN tasks task ON task.id = deliverable.task_id
                JOIN missions mission ON mission.id = task.mission_id
                JOIN room_memberships membership ON membership.room_id = mission.room_id
                WHERE deliverable.corp_id = $1
                  AND task.mission_id = $2
                  AND membership.actor_id = $3
                ORDER BY deliverable.created_at DESC, deliverable.id
                "#,
            )
            .bind(corp_id)
            .bind(mission_id)
            .bind(viewer_actor_id)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(map_source_deliverable)
            .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };

        tx.commit().await?;
        Ok(Some(FactoryPublicationContext {
            work_item,
            publication,
            source_deliverables,
        }))
    }

    pub async fn pull_request_publication_for_work_item(
        &self,
        corp_id: Uuid,
        viewer_actor_id: Uuid,
        work_item_id: Uuid,
    ) -> Result<Option<PullRequestPublication>> {
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, corp_id, viewer_actor_id).await?;
        let publication =
            publication_for_work_item_viewer_tx(&mut tx, corp_id, viewer_actor_id, work_item_id)
                .await?;
        tx.commit().await?;
        Ok(publication.map(|(publication, _)| publication))
    }

    pub async fn start_pull_request_publication(
        &self,
        input: StartPullRequestPublicationInput,
    ) -> Result<PullRequestPublicationOutcome> {
        let normalized = normalize_start_input(input)?;
        let now = Utc::now();
        let lease_expires_at = now + Duration::seconds(normalized.lease_seconds);
        let operation_request = start_operation_request(&normalized);
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, normalized.corp_id, normalized.actor_id).await?;
        revalidate_publication_publisher_credential_tx(
            &mut tx,
            normalized.corp_id,
            &normalized.publisher_id,
            &normalized.publisher_credential_hash,
        )
        .await?;
        ensure_actor_role_tx(
            &mut tx,
            normalized.corp_id,
            normalized.actor_id,
            &normalized.actor_role,
        )
        .await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!(
                    "publication:idempotency:{}:{}",
                    normalized.corp_id, normalized.idempotency_key
                ),
                format!(
                    "publication:factory:{}:{}",
                    normalized.corp_id, normalized.work_item_id
                ),
                format!(
                    "publication:effect:{}:{}",
                    normalized.corp_id, normalized.effect_key
                ),
                format!(
                    "publication:branch:{}:{}:{}",
                    normalized.corp_id, normalized.target_repository, normalized.branch
                ),
            ],
        )
        .await?;

        if let Some(operation) =
            publication_operation_tx(&mut tx, normalized.corp_id, &normalized.idempotency_key)
                .await?
        {
            ensure_publication_operation_matches(
                &operation,
                "start",
                normalized.actor_id,
                None,
                &operation_request,
            )?;
            let (publication, current_token) =
                publication_by_id_tx(&mut tx, normalized.corp_id, operation.publication_id, false)
                    .await?
                    .context("idempotent publication start references a missing publication")?;
            assert_publication_room_membership_tx(&mut tx, &publication, normalized.actor_id)
                .await?;
            let publisher_token =
                replayable_publication_token(&publication, current_token, &operation, now);
            let busy = publication.state != PullRequestPublicationState::Published
                && publisher_token.is_none()
                && publication
                    .publisher_lease_expires_at
                    .is_some_and(|expiry| expiry > now);
            tx.commit().await?;
            return Ok(PullRequestPublicationOutcome {
                publication,
                publisher_token,
                events: Vec::new(),
                replayed: true,
                busy,
            });
        }

        let existing = publication_collision_tx(
            &mut tx,
            normalized.corp_id,
            normalized.work_item_id,
            &normalized.effect_key,
            &normalized.target_repository,
            &normalized.branch,
        )
        .await?;
        if let Some((publication, current_token)) = existing {
            ensure_publication_matches_start(&publication, &normalized)?;
            assert_publication_room_membership_tx(&mut tx, &publication, normalized.actor_id)
                .await?;
            if publication.state == PullRequestPublicationState::Published {
                record_publication_operation_tx(
                    &mut tx,
                    NewPublicationOperation {
                        corp_id: normalized.corp_id,
                        idempotency_key: &normalized.idempotency_key,
                        publication_id: publication.id,
                        actor_id: normalized.actor_id,
                        operation: "start",
                        resulting_version: publication.version,
                        publisher_token: None,
                        request: &operation_request,
                    },
                )
                .await?;
                tx.commit().await?;
                return Ok(PullRequestPublicationOutcome {
                    publication,
                    publisher_token: None,
                    events: Vec::new(),
                    replayed: true,
                    busy: false,
                });
            }
            validate_publication_prerequisites(
                &mut tx,
                &PublicationPrerequisiteRequest::from_start(&normalized),
                true,
            )
            .await?;
            if publication
                .publisher_lease_expires_at
                .is_some_and(|expiry| expiry > now)
                && current_token.is_some()
            {
                tx.commit().await?;
                return Ok(PullRequestPublicationOutcome {
                    publication,
                    publisher_token: None,
                    events: Vec::new(),
                    replayed: true,
                    busy: true,
                });
            }

            sqlx::query(
                r#"
                UPDATE pull_request_publication_attempts
                SET state = 'abandoned',
                    failure_detail = COALESCE(
                        failure_detail,
                        'Publisher lease expired before the attempt completed.'
                    ),
                    finished_at = COALESCE(finished_at, now())
                WHERE publication_id = $1
                  AND corp_id = $2
                  AND state = 'running'
                "#,
            )
            .bind(publication.id)
            .bind(normalized.corp_id)
            .execute(&mut *tx)
            .await?;

            let publisher_token = Uuid::new_v4();
            let authorization = publication_authorization(&normalized, now);
            let row = sqlx::query(&format!(
                r#"
                UPDATE pull_request_publications publication
                SET state = CASE
                        WHEN state = 'requested' THEN 'publishing'
                        ELSE state
                    END,
                    version = version + 1,
                    attempt_count = attempt_count + 1,
                    publisher_id = $1,
                    publisher_token = $2,
                    publisher_lease_expires_at = $3,
                    failure_detail = NULL,
                    updated_at = now()
                WHERE id = $4 AND corp_id = $5
                RETURNING {}
                "#,
                publication_returning_columns()
            ))
            .bind(&normalized.publisher_id)
            .bind(publisher_token)
            .bind(lease_expires_at)
            .bind(publication.id)
            .bind(normalized.corp_id)
            .fetch_one(&mut *tx)
            .await?;
            let recovered = map_pull_request_publication(row)?;
            insert_publication_attempt_tx(
                &mut tx,
                &recovered,
                normalized.actor_id,
                normalized.authorization_id,
                &authorization,
                &normalized.publisher_id,
            )
            .await?;
            record_publication_operation_tx(
                &mut tx,
                NewPublicationOperation {
                    corp_id: normalized.corp_id,
                    idempotency_key: &normalized.idempotency_key,
                    publication_id: recovered.id,
                    actor_id: normalized.actor_id,
                    operation: "start",
                    resulting_version: recovered.version,
                    publisher_token: Some(publisher_token),
                    request: &operation_request,
                },
            )
            .await?;
            let event = publication_event_tx(
                &mut tx,
                &recovered,
                normalized.actor_id,
                "factory.publication_attempt_started",
                json!({
                    "factory_work_item_id": recovered.factory_work_item_id,
                    "attempt": recovered.attempt_count,
                    "publisher_id": recovered.publisher_id,
                    "state": recovered.state.as_str(),
                    "recovered": true
                }),
            )
            .await?;
            tx.commit().await?;
            return Ok(PullRequestPublicationOutcome {
                publication: recovered,
                publisher_token: Some(publisher_token),
                events: event.into_iter().collect(),
                replayed: false,
                busy: false,
            });
        }

        let prerequisites = validate_publication_prerequisites(
            &mut tx,
            &PublicationPrerequisiteRequest::from_start(&normalized),
            false,
        )
        .await?;
        assert_room_membership_tx(
            &mut tx,
            normalized.corp_id,
            prerequisites.room_id,
            normalized.actor_id,
        )
        .await?;
        let publication_id = Uuid::new_v4();
        let publisher_token = Uuid::new_v4();
        let authorization = publication_authorization(&normalized, now);
        let provenance = json!({
            "schema_version": if prerequisites.checkpoint.is_some() { 3 } else { 2 },
            "source_issue": {
                "number": prerequisites.work_item.source_issue_number,
                "node_id": prerequisites.work_item.source_issue_node_id,
                "url": prerequisites.work_item.source_issue_url,
                "revision": prerequisites.effective_source_revision,
                "claimed_revision": prerequisites.work_item.source_revision,
                "recovery_id": prerequisites.source_recovery_id,
            },
            "factory_work_item_id": prerequisites.work_item.id,
            "mission_id": prerequisites.mission_id,
            "task_ids": prerequisites.task_ids,
            "run_ids": prerequisites.run_ids,
            "verification_evidence_ids": prerequisites.evidence_ids,
            "verification_sha256": prerequisites.verification_sha256,
            "checkpoint": prerequisites.checkpoint.as_ref().map(|checkpoint| &checkpoint.provenance),
            "deliverable": {
                "id": normalized.source_deliverable_id,
                "artifact_id": prerequisites.artifact_id,
                "sha256": prerequisites.deliverable_sha256,
                "base_commit": prerequisites.base_commit,
                "head_commit": prerequisites.commit_sha,
                "source_branch": prerequisites.source_branch,
            },
            "target": {
                "repository": normalized.target_repository,
                "base_ref": normalized.base_ref,
                "branch": normalized.branch,
                "commit": prerequisites.commit_sha,
            },
            "pull_request": Value::Null,
            "project": {
                "owner": prerequisites.work_item.source_project_owner,
                "number": prerequisites.work_item.source_project_number,
                "item_id": prerequisites.work_item.source_project_item_id,
                "status_before": prerequisites.project_status_before,
                "review_status": prerequisites.review_status,
            },
            "authorization_snapshot": authorization,
            "effects": {
                "effect_key": normalized.effect_key,
                "auto_merge": false,
                "merge": false,
                "deploy": false,
            }
        });
        let row = sqlx::query(&format!(
            r#"
            INSERT INTO pull_request_publications
                (id, corp_id, factory_work_item_id, mission_id, source_deliverable_id,
                 artifact_id, task_id, run_id, source_issue_number, source_issue_url,
                 target_repository, base_ref, branch, commit_sha, title, body,
                 actor_id, authorization_id, authorization_snapshot, effect_key, idempotency_key,
                 state, version, attempt_count, publisher_id, publisher_token,
                 publisher_lease_expires_at, project_owner, project_number,
                 project_item_id, project_status_before, auto_merge_enabled,
                 merge_authorized, deployment_authorized, provenance)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 $14, $15, $16, $17, $18, $19, $20, $21, 'publishing', 1, 1,
                 $22, $23, $24, $25, $26, $27, $28, FALSE, FALSE, FALSE, $29)
            RETURNING {}
            "#,
            publication_returning_columns()
        ))
        .bind(publication_id)
        .bind(normalized.corp_id)
        .bind(prerequisites.work_item.id)
        .bind(prerequisites.mission_id)
        .bind(normalized.source_deliverable_id)
        .bind(prerequisites.artifact_id)
        .bind(prerequisites.task_id)
        .bind(prerequisites.run_id)
        .bind(prerequisites.work_item.source_issue_number)
        .bind(&prerequisites.work_item.source_issue_url)
        .bind(&normalized.target_repository)
        .bind(&normalized.base_ref)
        .bind(&normalized.branch)
        .bind(&prerequisites.commit_sha)
        .bind(&normalized.title)
        .bind(&normalized.body)
        .bind(normalized.actor_id)
        .bind(normalized.authorization_id)
        .bind(&authorization)
        .bind(&normalized.effect_key)
        .bind(&normalized.idempotency_key)
        .bind(&normalized.publisher_id)
        .bind(publisher_token)
        .bind(lease_expires_at)
        .bind(&prerequisites.work_item.source_project_owner)
        .bind(prerequisites.work_item.source_project_number)
        .bind(&prerequisites.work_item.source_project_item_id)
        .bind(&prerequisites.project_status_before)
        .bind(&provenance)
        .fetch_one(&mut *tx)
        .await?;
        let publication = map_pull_request_publication(row)?;
        insert_publication_attempt_tx(
            &mut tx,
            &publication,
            normalized.actor_id,
            normalized.authorization_id,
            &authorization,
            &normalized.publisher_id,
        )
        .await?;
        record_publication_operation_tx(
            &mut tx,
            NewPublicationOperation {
                corp_id: normalized.corp_id,
                idempotency_key: &normalized.idempotency_key,
                publication_id,
                actor_id: normalized.actor_id,
                operation: "start",
                resulting_version: publication.version,
                publisher_token: Some(publisher_token),
                request: &operation_request,
            },
        )
        .await?;

        let factory_version: i64 = sqlx::query_scalar(
            r#"
            UPDATE factory_work_items
            SET state = 'publishing',
                version = version + 1,
                failure_detail = NULL,
                updated_at = now()
            WHERE id = $1 AND corp_id = $2 AND state = 'verified'
            RETURNING version
            "#,
        )
        .bind(prerequisites.work_item.id)
        .bind(normalized.corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let mut events = Vec::new();
        if let Some(event) = append_event_tx(
            &mut tx,
            NewEvent {
                room_id: Some(prerequisites.room_id),
                aggregate_version: factory_version,
                correlation_id: Some(prerequisites.mission_id),
                ..NewEvent::new(
                    normalized.corp_id,
                    Some(normalized.actor_id),
                    "factory.state_changed",
                    "factory_work_item",
                    prerequisites.work_item.id,
                    format!(
                        "factory:{}:state:publishing:{factory_version}",
                        prerequisites.work_item.id
                    ),
                    json!({
                        "previous_state": "verified",
                        "state": "publishing",
                        "mission_id": prerequisites.mission_id,
                        "publication_id": publication.id
                    }),
                )
            },
        )
        .await?
        {
            events.push(event);
        }
        if let Some(event) = publication_event_tx(
            &mut tx,
            &publication,
            normalized.actor_id,
            "factory.publication_requested",
            json!({
                "factory_work_item_id": publication.factory_work_item_id,
                "source_deliverable_id": publication.source_deliverable_id,
                "target_repository": publication.target_repository,
                "base_ref": publication.base_ref,
                "branch": publication.branch,
                "commit_sha": publication.commit_sha,
                "attempt": publication.attempt_count,
                "state": publication.state.as_str(),
                "auto_merge": false,
                "merge": false,
                "deploy": false
            }),
        )
        .await?
        {
            events.push(event);
        }
        tx.commit().await?;
        Ok(PullRequestPublicationOutcome {
            publication,
            publisher_token: Some(publisher_token),
            events,
            replayed: false,
            busy: false,
        })
    }

    pub async fn renew_pull_request_publication(
        &self,
        input: RenewPullRequestPublicationInput,
    ) -> Result<PullRequestPublicationOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!("expected publication version must be positive"));
        }
        let idempotency_key = normalize_factory_identifier(
            &input.idempotency_key,
            "publication idempotency key",
            500,
        )?;
        let publisher_id =
            normalize_factory_identifier(&input.publisher_id, "trusted publisher id", 160)?;
        let publisher_credential_hash =
            normalize_publication_publisher_credential_hash(&input.publisher_credential_hash)?;
        let lease_seconds = validate_publication_lease_seconds(input.lease_seconds)?;
        let operation_request = json!({
            "publication_id": input.publication_id,
            "publisher_id": publisher_id,
            "expected_version": input.expected_version,
            "lease_seconds": lease_seconds
        });
        let now = Utc::now();
        let lease_expires_at = now + Duration::seconds(lease_seconds);
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        revalidate_publication_publisher_credential_tx(
            &mut tx,
            input.corp_id,
            &publisher_id,
            &publisher_credential_hash,
        )
        .await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!(
                    "publication:idempotency:{}:{idempotency_key}",
                    input.corp_id
                ),
                format!(
                    "publication:item:{}:{}",
                    input.corp_id, input.publication_id
                ),
            ],
        )
        .await?;
        if let Some(operation) =
            publication_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_publication_operation_matches(
                &operation,
                "renew",
                input.actor_id,
                Some(input.publisher_token),
                &operation_request,
            )?;
            let (publication, _) =
                publication_by_id_tx(&mut tx, input.corp_id, input.publication_id, true)
                    .await?
                    .context("idempotent publication renewal references a missing publication")?;
            revalidate_publication_authority_tx(&mut tx, &publication, input.actor_id).await?;
            let (publication, current_token) =
                publication_by_id_tx(&mut tx, input.corp_id, input.publication_id, false)
                    .await?
                    .context("publication disappeared after renewal revalidation")?;
            let publisher_token =
                replayable_publication_token(&publication, current_token, &operation, now);
            tx.commit().await?;
            return Ok(PullRequestPublicationOutcome {
                publication,
                publisher_token,
                events: Vec::new(),
                replayed: true,
                busy: false,
            });
        }
        let (current, current_token) =
            publication_by_id_tx(&mut tx, input.corp_id, input.publication_id, true)
                .await?
                .context("pull-request publication not found")?;
        ensure_active_publication_control_tx(
            &mut tx,
            &current,
            current_token,
            ActivePublicationControl {
                actor_id: input.actor_id,
                publisher_id: &publisher_id,
                presented_token: input.publisher_token,
                expected_version: input.expected_version,
                now,
            },
        )
        .await?;
        revalidate_publication_authority_tx(&mut tx, &current, input.actor_id).await?;
        let row = sqlx::query(&format!(
            r#"
            UPDATE pull_request_publications publication
            SET version = version + 1,
                publisher_lease_expires_at = $1,
                updated_at = now()
            WHERE id = $2 AND corp_id = $3
            RETURNING {}
            "#,
            publication_returning_columns()
        ))
        .bind(lease_expires_at)
        .bind(input.publication_id)
        .bind(input.corp_id)
        .fetch_one(&mut *tx)
        .await?;
        let publication = map_pull_request_publication(row)?;
        record_publication_operation_tx(
            &mut tx,
            NewPublicationOperation {
                corp_id: input.corp_id,
                idempotency_key: &idempotency_key,
                publication_id: publication.id,
                actor_id: input.actor_id,
                operation: "renew",
                resulting_version: publication.version,
                publisher_token: Some(input.publisher_token),
                request: &operation_request,
            },
        )
        .await?;
        let event = publication_event_tx(
            &mut tx,
            &publication,
            input.actor_id,
            "factory.publication_lease_renewed",
            json!({
                "attempt": publication.attempt_count,
                "publisher_id": publication.publisher_id,
                "lease_expires_at": publication.publisher_lease_expires_at,
                "state": publication.state.as_str()
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(PullRequestPublicationOutcome {
            publication,
            publisher_token: Some(input.publisher_token),
            events: event.into_iter().collect(),
            replayed: false,
            busy: false,
        })
    }

    pub async fn record_pull_request_publication_checkpoint(
        &self,
        input: RecordPullRequestPublicationCheckpointInput,
    ) -> Result<PullRequestPublicationOutcome> {
        if input.expected_version <= 0 {
            return Err(anyhow!("expected publication version must be positive"));
        }
        let idempotency_key = normalize_factory_identifier(
            &input.idempotency_key,
            "publication idempotency key",
            500,
        )?;
        let publisher_id =
            normalize_factory_identifier(&input.publisher_id, "trusted publisher id", 160)?;
        let publisher_credential_hash =
            normalize_publication_publisher_credential_hash(&input.publisher_credential_hash)?;
        let (operation, checkpoint, checkpoint_request) = normalize_checkpoint(input.checkpoint)?;
        let operation_request = json!({
            "publisher_id": publisher_id,
            "checkpoint": checkpoint_request
        });
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        assert_actor_scope_tx(&mut tx, input.corp_id, input.actor_id).await?;
        revalidate_publication_publisher_credential_tx(
            &mut tx,
            input.corp_id,
            &publisher_id,
            &publisher_credential_hash,
        )
        .await?;
        lock_factory_keys_tx(
            &mut tx,
            &[
                format!(
                    "publication:idempotency:{}:{idempotency_key}",
                    input.corp_id
                ),
                format!(
                    "publication:item:{}:{}",
                    input.corp_id, input.publication_id
                ),
            ],
        )
        .await?;
        if let Some(existing_operation) =
            publication_operation_tx(&mut tx, input.corp_id, &idempotency_key).await?
        {
            ensure_publication_operation_matches(
                &existing_operation,
                operation,
                input.actor_id,
                Some(input.publisher_token),
                &operation_request,
            )?;
            let (publication, current_token) =
                publication_by_id_tx(&mut tx, input.corp_id, input.publication_id, false)
                    .await?
                    .context(
                        "idempotent publication checkpoint references a missing publication",
                    )?;
            let publisher_token =
                replayable_publication_token(&publication, current_token, &existing_operation, now);
            tx.commit().await?;
            return Ok(PullRequestPublicationOutcome {
                publication,
                publisher_token,
                events: Vec::new(),
                replayed: true,
                busy: false,
            });
        }
        let (current, current_token) =
            publication_by_id_tx(&mut tx, input.corp_id, input.publication_id, true)
                .await?
                .context("pull-request publication not found")?;
        ensure_active_publication_control_tx(
            &mut tx,
            &current,
            current_token,
            ActivePublicationControl {
                actor_id: input.actor_id,
                publisher_id: &publisher_id,
                presented_token: input.publisher_token,
                expected_version: input.expected_version,
                now,
            },
        )
        .await?;

        let current = if matches!(
            &checkpoint,
            PullRequestPublicationCheckpointInput::Failed { .. }
        ) {
            // Failure records close this exact owned attempt; they advance no
            // external effect. Retain the Corp, credential, actor, token, version
            // and lease checks above even when effect authority was revoked.
            current
        } else {
            // A lease cannot substitute for current source/review/actor authority.
            revalidate_publication_authority_tx(&mut tx, &current, input.actor_id).await?;
            // Return the persisted upgrade, not the pre-validation copy.
            publication_by_id_tx(&mut tx, input.corp_id, input.publication_id, false)
                .await?
                .context("publication disappeared after authority revalidation")?
                .0
        };

        let mut events = Vec::new();
        let (publication, replayed) = match checkpoint {
            PullRequestPublicationCheckpointInput::BranchPushed { commit_sha } => {
                if publication_state_rank(current.state)
                    >= publication_state_rank(PullRequestPublicationState::BranchPushed)
                {
                    if current.commit_sha != commit_sha || current.branch_pushed_at.is_none() {
                        return Err(anyhow!(
                            "conflict: publication branch checkpoint does not match the persisted branch"
                        ));
                    }
                    record_publication_operation_tx(
                        &mut tx,
                        NewPublicationOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            publication_id: current.id,
                            actor_id: input.actor_id,
                            operation,
                            resulting_version: current.version,
                            publisher_token: Some(input.publisher_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    (current, true)
                } else {
                    if current.state != PullRequestPublicationState::Publishing
                        || current.commit_sha != commit_sha
                    {
                        return Err(anyhow!(
                            "conflict: publication cannot record this branch checkpoint"
                        ));
                    }
                    let row = sqlx::query(&format!(
                        r#"
                        UPDATE pull_request_publications publication
                        SET state = 'branch_pushed',
                            version = version + 1,
                            branch_pushed_at = now(),
                            failure_detail = NULL,
                            updated_at = now()
                        WHERE id = $1 AND corp_id = $2
                        RETURNING {}
                        "#,
                        publication_returning_columns()
                    ))
                    .bind(current.id)
                    .bind(input.corp_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    let publication = map_pull_request_publication(row)?;
                    record_publication_operation_tx(
                        &mut tx,
                        NewPublicationOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            publication_id: publication.id,
                            actor_id: input.actor_id,
                            operation,
                            resulting_version: publication.version,
                            publisher_token: Some(input.publisher_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    if let Some(event) = publication_event_tx(
                        &mut tx,
                        &publication,
                        input.actor_id,
                        "factory.publication_branch_pushed",
                        json!({
                            "repository": publication.target_repository,
                            "branch": publication.branch,
                            "commit_sha": publication.commit_sha,
                            "attempt": publication.attempt_count
                        }),
                    )
                    .await?
                    {
                        events.push(event);
                    }
                    (publication, false)
                }
            }
            PullRequestPublicationCheckpointInput::PullRequestCreated {
                number,
                node_id,
                url,
                state,
                draft,
                title,
                body,
                head_ref,
                base_ref,
                head_sha,
                head_repository_owner,
                is_cross_repository,
                auto_merge_enabled,
            } => {
                validate_pull_request_identity(
                    &current,
                    number,
                    &node_id,
                    &url,
                    &state,
                    &title,
                    &body,
                    &head_ref,
                    &base_ref,
                    &head_sha,
                    &head_repository_owner,
                    is_cross_repository,
                    auto_merge_enabled,
                )?;
                if publication_state_rank(current.state)
                    >= publication_state_rank(PullRequestPublicationState::PullRequestCreated)
                {
                    ensure_pull_request_identity_matches(
                        &current,
                        number,
                        &node_id,
                        &url,
                        &state,
                        draft,
                        &base_ref,
                        &head_sha,
                        &head_repository_owner,
                        is_cross_repository,
                    )?;
                    record_publication_operation_tx(
                        &mut tx,
                        NewPublicationOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            publication_id: current.id,
                            actor_id: input.actor_id,
                            operation,
                            resulting_version: current.version,
                            publisher_token: Some(input.publisher_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    (current, true)
                } else {
                    if current.state != PullRequestPublicationState::BranchPushed {
                        return Err(anyhow!(
                            "conflict: pull request cannot be recorded before the branch is durable"
                        ));
                    }
                    let mut provenance = current.provenance.clone();
                    provenance["pull_request"] = json!({
                        "number": number,
                        "node_id": node_id,
                        "url": url,
                        "state": state,
                        "draft": draft,
                        "title": title,
                        "body": body,
                        "base_ref": base_ref,
                        "head_sha": head_sha,
                        "head_repository_owner": head_repository_owner,
                        "is_cross_repository": false,
                        "auto_merge": false
                    });
                    let row = sqlx::query(&format!(
                        r#"
                        UPDATE pull_request_publications publication
                        SET state = 'pull_request_created',
                            version = version + 1,
                            pull_request_number = $1,
                            pull_request_node_id = $2,
                            pull_request_url = $3,
                            pull_request_state = $4,
                            pull_request_draft = $5,
                            pull_request_base_ref = $6,
                            pull_request_head_sha = $7,
                            pull_request_head_repository_owner = $8,
                            pull_request_is_cross_repository = $9,
                            failure_detail = NULL,
                            provenance = $10,
                            updated_at = now()
                        WHERE id = $11 AND corp_id = $12
                        RETURNING {}
                        "#,
                        publication_returning_columns()
                    ))
                    .bind(number)
                    .bind(&node_id)
                    .bind(&url)
                    .bind(&state)
                    .bind(draft)
                    .bind(&base_ref)
                    .bind(&head_sha)
                    .bind(&head_repository_owner)
                    .bind(is_cross_repository)
                    .bind(provenance)
                    .bind(current.id)
                    .bind(input.corp_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    let publication = map_pull_request_publication(row)?;
                    record_publication_operation_tx(
                        &mut tx,
                        NewPublicationOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            publication_id: publication.id,
                            actor_id: input.actor_id,
                            operation,
                            resulting_version: publication.version,
                            publisher_token: Some(input.publisher_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    if let Some(event) = publication_event_tx(
                        &mut tx,
                        &publication,
                        input.actor_id,
                        "factory.pull_request_created",
                        json!({
                            "pull_request_number": publication.pull_request_number,
                            "pull_request_node_id": publication.pull_request_node_id,
                            "pull_request_url": publication.pull_request_url,
                            "pull_request_state": publication.pull_request_state,
                            "draft": publication.pull_request_draft,
                            "base_ref": publication.pull_request_base_ref,
                            "head_sha": publication.pull_request_head_sha,
                            "head_repository_owner": publication.pull_request_head_repository_owner,
                            "is_cross_repository": publication.pull_request_is_cross_repository,
                            "auto_merge": false,
                            "attempt": publication.attempt_count
                        }),
                    )
                    .await?
                    {
                        events.push(event);
                    }
                    (publication, false)
                }
            }
            PullRequestPublicationCheckpointInput::Published {
                project_status,
                project_field_id,
                project_option_id,
            } => {
                let expected_status = current
                    .provenance
                    .pointer("/project/review_status")
                    .and_then(Value::as_str)
                    .context("publication provenance omitted Project review status")?;
                if project_status != expected_status {
                    return Err(anyhow!(
                        "Project status {project_status} does not match authorized review status {expected_status}"
                    ));
                }
                normalize_factory_identifier(&project_field_id, "GitHub Project field id", 200)?;
                normalize_factory_identifier(&project_option_id, "GitHub Project option id", 200)?;
                if current.state == PullRequestPublicationState::Published {
                    if current.project_status_after.as_deref() != Some(project_status.as_str()) {
                        return Err(anyhow!(
                            "conflict: publication already completed with a different Project status"
                        ));
                    }
                    record_publication_operation_tx(
                        &mut tx,
                        NewPublicationOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            publication_id: current.id,
                            actor_id: input.actor_id,
                            operation,
                            resulting_version: current.version,
                            publisher_token: Some(input.publisher_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    (current, true)
                } else {
                    if current.state != PullRequestPublicationState::PullRequestCreated
                        || current.pull_request_number.is_none()
                        || current.pull_request_url.is_none()
                    {
                        return Err(anyhow!(
                            "conflict: Project review status cannot advance before the pull request exists"
                        ));
                    }
                    let mut provenance = current.provenance.clone();
                    provenance["project"]["status_after"] = Value::String(project_status.clone());
                    provenance["project"]["field_id"] = Value::String(project_field_id);
                    provenance["project"]["option_id"] = Value::String(project_option_id);
                    let row = sqlx::query(&format!(
                        r#"
                        UPDATE pull_request_publications publication
                        SET state = 'published',
                            version = version + 1,
                            project_status_after = $1,
                            project_status_updated_at = now(),
                            publisher_id = NULL,
                            publisher_token = NULL,
                            publisher_lease_expires_at = NULL,
                            failure_detail = NULL,
                            provenance = $2,
                            updated_at = now()
                        WHERE id = $3 AND corp_id = $4
                        RETURNING {}
                        "#,
                        publication_returning_columns()
                    ))
                    .bind(&project_status)
                    .bind(provenance)
                    .bind(current.id)
                    .bind(input.corp_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    let publication = map_pull_request_publication(row)?;
                    sqlx::query(
                        r#"
                        UPDATE pull_request_publication_attempts
                        SET state = 'published', finished_at = now(), failure_detail = NULL
                        WHERE publication_id = $1
                          AND corp_id = $2
                          AND attempt = $3
                          AND state = 'running'
                        "#,
                    )
                    .bind(publication.id)
                    .bind(input.corp_id)
                    .bind(publication.attempt_count)
                    .execute(&mut *tx)
                    .await?;
                    let factory_version: i64 = sqlx::query_scalar(
                        r#"
                        UPDATE factory_work_items
                        SET state = 'published',
                            version = version + 1,
                            failure_detail = NULL,
                            updated_at = now()
                        WHERE id = $1 AND corp_id = $2 AND state = 'publishing'
                        RETURNING version
                        "#,
                    )
                    .bind(publication.factory_work_item_id)
                    .bind(input.corp_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    sqlx::query(
                        r#"
                        UPDATE source_deliverables
                        SET integration_state = 'published'
                        WHERE id = $1 AND corp_id = $2
                        "#,
                    )
                    .bind(publication.source_deliverable_id)
                    .bind(input.corp_id)
                    .execute(&mut *tx)
                    .await?;
                    record_publication_operation_tx(
                        &mut tx,
                        NewPublicationOperation {
                            corp_id: input.corp_id,
                            idempotency_key: &idempotency_key,
                            publication_id: publication.id,
                            actor_id: input.actor_id,
                            operation,
                            resulting_version: publication.version,
                            publisher_token: Some(input.publisher_token),
                            request: &operation_request,
                        },
                    )
                    .await?;
                    let room_id = publication_room_id_tx(
                        &mut tx,
                        publication.corp_id,
                        publication.mission_id,
                    )
                    .await?;
                    if let Some(event) = append_event_tx(
                        &mut tx,
                        NewEvent {
                            room_id: Some(room_id),
                            aggregate_version: factory_version,
                            correlation_id: Some(publication.mission_id),
                            ..NewEvent::new(
                                publication.corp_id,
                                Some(input.actor_id),
                                "factory.state_changed",
                                "factory_work_item",
                                publication.factory_work_item_id,
                                format!(
                                    "factory:{}:state:published:{factory_version}",
                                    publication.factory_work_item_id
                                ),
                                json!({
                                    "previous_state": "publishing",
                                    "state": "published",
                                    "mission_id": publication.mission_id,
                                    "publication_id": publication.id,
                                    "pull_request_url": publication.pull_request_url
                                }),
                            )
                        },
                    )
                    .await?
                    {
                        events.push(event);
                    }
                    if let Some(event) = publication_event_tx(
                        &mut tx,
                        &publication,
                        input.actor_id,
                        "factory.publication_completed",
                        json!({
                            "pull_request_number": publication.pull_request_number,
                            "pull_request_url": publication.pull_request_url,
                            "project_status": publication.project_status_after,
                            "attempt": publication.attempt_count,
                            "auto_merge": false,
                            "merge": false,
                            "deploy": false
                        }),
                    )
                    .await?
                    {
                        events.push(event);
                    }
                    (publication, false)
                }
            }
            PullRequestPublicationCheckpointInput::Failed { failure_detail } => {
                if current.state == PullRequestPublicationState::Published {
                    return Err(anyhow!(
                        "conflict: a published pull request cannot be marked failed"
                    ));
                }
                let row = sqlx::query(&format!(
                    r#"
                    UPDATE pull_request_publications publication
                    SET version = version + 1,
                        publisher_id = NULL,
                        publisher_token = NULL,
                        publisher_lease_expires_at = NULL,
                        failure_detail = $1,
                        updated_at = now()
                    WHERE id = $2 AND corp_id = $3
                    RETURNING {}
                    "#,
                    publication_returning_columns()
                ))
                .bind(&failure_detail)
                .bind(current.id)
                .bind(input.corp_id)
                .fetch_one(&mut *tx)
                .await?;
                let publication = map_pull_request_publication(row)?;
                sqlx::query(
                    r#"
                    UPDATE pull_request_publication_attempts
                    SET state = 'failed', failure_detail = $1, finished_at = now()
                    WHERE publication_id = $2
                      AND corp_id = $3
                      AND attempt = $4
                      AND state = 'running'
                    "#,
                )
                .bind(&failure_detail)
                .bind(publication.id)
                .bind(input.corp_id)
                .bind(publication.attempt_count)
                .execute(&mut *tx)
                .await?;
                record_publication_operation_tx(
                    &mut tx,
                    NewPublicationOperation {
                        corp_id: input.corp_id,
                        idempotency_key: &idempotency_key,
                        publication_id: publication.id,
                        actor_id: input.actor_id,
                        operation,
                        resulting_version: publication.version,
                        publisher_token: Some(input.publisher_token),
                        request: &operation_request,
                    },
                )
                .await?;
                if let Some(event) = publication_event_tx(
                    &mut tx,
                    &publication,
                    input.actor_id,
                    "factory.publication_failed",
                    json!({
                        "state": publication.state.as_str(),
                        "attempt": publication.attempt_count,
                        "failure_detail": publication.failure_detail
                    }),
                )
                .await?
                {
                    events.push(event);
                }
                (publication, false)
            }
        };
        let publisher_token = (publication.state != PullRequestPublicationState::Published
            && publication.publisher_lease_expires_at.is_some())
        .then_some(input.publisher_token);
        tx.commit().await?;
        Ok(PullRequestPublicationOutcome {
            publication,
            publisher_token,
            events,
            replayed,
            busy: false,
        })
    }
}

fn normalize_start_input(
    mut input: StartPullRequestPublicationInput,
) -> Result<StartPullRequestPublicationInput> {
    let repository = input.target_repository.trim();
    let mut parts = repository.split('/');
    let owner = parts
        .next()
        .context("publication target repository omitted owner")?;
    let name = parts
        .next()
        .context("publication target repository omitted name")?;
    if parts.next().is_some() {
        return Err(anyhow!(
            "publication target repository must use owner/name form"
        ));
    }
    input.target_repository = format!(
        "{}/{}",
        normalize_github_component(owner, "publication repository owner", 100)?,
        normalize_github_component(name, "publication repository name", 100)?
    );
    input.base_ref = normalize_factory_identifier(&input.base_ref, "publication base ref", 240)?;
    validate_factory_base_ref(&input.base_ref)?;
    input.branch = normalize_factory_identifier(&input.branch, "publication branch", 500)?;
    validate_factory_branch_ref(&input.branch)?;
    if input.branch == input.base_ref {
        return Err(anyhow!(
            "publication branch must differ from the target base ref"
        ));
    }
    input.title = normalize_factory_text(&input.title, "pull request title", 256)?;
    input.body = normalize_publication_body(&input.body)?;
    input.authorization_reason = normalize_factory_text(
        &input.authorization_reason,
        "publication authorization reason",
        2_000,
    )?;
    input.effect_key =
        normalize_factory_identifier(&input.effect_key, "publication effect key", 500)?;
    input.idempotency_key =
        normalize_factory_identifier(&input.idempotency_key, "publication idempotency key", 500)?;
    input.publisher_id =
        normalize_factory_identifier(&input.publisher_id, "trusted publisher id", 160)?;
    input.publisher_credential_hash =
        normalize_publication_publisher_credential_hash(&input.publisher_credential_hash)?;
    input.actor_role = normalize_factory_identifier(&input.actor_role, "actor role", 40)?;
    input.lease_seconds = validate_publication_lease_seconds(input.lease_seconds)?;
    Ok(input)
}

fn normalize_publication_body(value: &str) -> Result<String> {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("pull request body cannot be empty"));
    }
    if value.len() > 65_536 {
        return Err(anyhow!("pull request body cannot exceed 65536 bytes"));
    }
    if value.contains('\0') {
        return Err(anyhow!("pull request body cannot contain NUL bytes"));
    }
    Ok(value.to_owned())
}

fn validate_publication_lease_seconds(value: i64) -> Result<i64> {
    if !(5..=3_600).contains(&value) {
        return Err(anyhow!(
            "publication lease must be between 5 and 3600 seconds"
        ));
    }
    Ok(value)
}

fn normalize_publication_publisher_credential_hash(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "forbidden: invalid publication publisher credential"
        ));
    }
    Ok(value)
}

fn start_operation_request(input: &StartPullRequestPublicationInput) -> Value {
    json!({
        "work_item_id": input.work_item_id,
        "source_deliverable_id": input.source_deliverable_id,
        "target_repository": input.target_repository,
        "base_ref": input.base_ref,
        "branch": input.branch,
        "title": input.title,
        "body": input.body,
        "authorization_id": input.authorization_id,
        "authorization_reason": input.authorization_reason,
        "effect_key": input.effect_key,
        "publisher_id": input.publisher_id,
        "lease_seconds": input.lease_seconds
    })
}

fn publication_authorization(
    input: &StartPullRequestPublicationInput,
    authorized_at: chrono::DateTime<Utc>,
) -> Value {
    json!({
        "id": input.authorization_id,
        "kind": "explicit_human",
        "permission": "publish_pull_request",
        "actor_id": input.actor_id,
        "actor_role": input.actor_role,
        "reason": input.authorization_reason,
        "authorized_at": authorized_at,
        "auto_merge": false,
        "merge": false,
        "deploy": false
    })
}

async fn ensure_actor_role_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    actor_id: Uuid,
    expected_role: &str,
) -> Result<()> {
    let role: String = sqlx::query_scalar(
        "SELECT role FROM actors WHERE id = $1 AND corp_id = $2 AND kind = 'human'",
    )
    .bind(actor_id)
    .bind(corp_id)
    .fetch_one(&mut **tx)
    .await?;
    if role != expected_role {
        return Err(anyhow!(
            "publication authorization role changed from {expected_role} to {role}"
        ));
    }
    Ok(())
}

impl<'a> PublicationPrerequisiteRequest<'a> {
    fn from_start(input: &'a StartPullRequestPublicationInput) -> Self {
        Self {
            corp_id: input.corp_id,
            work_item_id: input.work_item_id,
            source_deliverable_id: input.source_deliverable_id,
            target_repository: &input.target_repository,
            base_ref: &input.base_ref,
            branch: &input.branch,
            body: &input.body,
        }
    }

    fn from_publication(publication: &'a PullRequestPublication) -> Self {
        Self {
            corp_id: publication.corp_id,
            work_item_id: publication.factory_work_item_id,
            source_deliverable_id: publication.source_deliverable_id,
            target_repository: &publication.target_repository,
            base_ref: &publication.base_ref,
            branch: &publication.branch,
            body: &publication.body,
        }
    }
}

async fn revalidate_publication_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    publication: &PullRequestPublication,
    actor_id: Uuid,
) -> Result<()> {
    let expected_role: String = sqlx::query_scalar(
        r#"
        SELECT authorization_snapshot->>'actor_role'
        FROM pull_request_publication_attempts
        WHERE publication_id = $1
          AND corp_id = $2
          AND attempt = $3
          AND actor_id = $4
          AND state = 'running'
        "#,
    )
    .bind(publication.id)
    .bind(publication.corp_id)
    .bind(publication.attempt_count)
    .bind(actor_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("active publication attempt omitted its authorization role")?;
    ensure_actor_role_tx(tx, publication.corp_id, actor_id, &expected_role).await?;
    let prerequisites = validate_publication_prerequisites(
        tx,
        &PublicationPrerequisiteRequest::from_publication(publication),
        true,
    )
    .await?;
    assert_room_membership_tx(tx, publication.corp_id, prerequisites.room_id, actor_id).await?;
    let provenance_deliverable_sha = publication
        .provenance
        .pointer("/deliverable/sha256")
        .and_then(Value::as_str);
    let provenance_verification_sha = publication
        .provenance
        .get("verification_sha256")
        .and_then(Value::as_str);
    let source_provenance = publication_source_provenance_state(
        &publication.provenance,
        &prerequisites.work_item.source_revision,
        &prerequisites.effective_source_revision,
        prerequisites.source_recovery_id,
    )?;
    if prerequisites.work_item.id != publication.factory_work_item_id
        || prerequisites.mission_id != publication.mission_id
        || prerequisites.artifact_id != publication.artifact_id
        || prerequisites.task_id != publication.task_id
        || prerequisites.run_id != publication.run_id
        || prerequisites.commit_sha != publication.commit_sha
        || provenance_deliverable_sha != Some(prerequisites.deliverable_sha256.as_str())
        || provenance_verification_sha != Some(prerequisites.verification_sha256.as_str())
        || source_provenance == PublicationSourceProvenanceState::Invalid
    {
        return Err(anyhow!(
            "conflict: publication authority no longer matches its verified provenance"
        ));
    }
    let source_upgraded = if source_provenance == PublicationSourceProvenanceState::Legacy {
        upgrade_publication_source_provenance(
            &publication.provenance,
            &prerequisites.work_item.source_revision,
            &prerequisites.effective_source_revision,
            prerequisites.source_recovery_id,
        )?
    } else {
        publication.provenance.clone()
    };
    // Older recovered-suspend publications predate checkpoint provenance. Only
    // reconstruct it after all current source, review and authority bindings pass.
    // Stamp schema 3 last so the source schema-1 upgrade cannot downgrade it.
    let upgraded = checkpoint_publication::revalidated_provenance(
        &source_upgraded,
        prerequisites
            .checkpoint
            .as_ref()
            .map(|checkpoint| &checkpoint.provenance),
    )?;
    if upgraded != publication.provenance {
        let updated = sqlx::query(
            r#"
            UPDATE pull_request_publications
            SET provenance = $1, updated_at = now()
            WHERE id = $2 AND corp_id = $3 AND provenance = $4
            "#,
        )
        .bind(upgraded)
        .bind(publication.id)
        .bind(publication.corp_id)
        .bind(&publication.provenance)
        .execute(&mut **tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(anyhow!(
                "conflict: legacy publication provenance changed during upgrade"
            ));
        }
    }
    Ok(())
}

async fn validate_publication_prerequisites(
    tx: &mut Transaction<'_, Postgres>,
    request: &PublicationPrerequisiteRequest<'_>,
    existing: bool,
) -> Result<PublicationPrerequisites> {
    let (work_item, _) = factory_work_item_tx(tx, request.corp_id, request.work_item_id, true)
        .await?
        .context("factory work item not found")?;
    let allowed_state = if existing {
        matches!(
            work_item.state,
            FactoryWorkItemState::Publishing | FactoryWorkItemState::Verified
        )
    } else {
        work_item.state == FactoryWorkItemState::Verified
    };
    if !allowed_state {
        return Err(anyhow!(
            "conflict: pull-request publication requires a verified factory work item, not {}",
            work_item.state.as_str()
        ));
    }
    let mission_id = work_item
        .mission_id
        .context("verified factory work item has no mission")?;
    ensure_factory_mission_verified_tx(tx, request.corp_id, mission_id).await?;
    let policy = work_item
        .policy
        .as_object()
        .context("factory policy snapshot must be an object")?;
    if policy.get("auto_merge").and_then(Value::as_bool) != Some(false) {
        return Err(anyhow!(
            "factory policy must explicitly disable auto_merge before publication"
        ));
    }
    let publication_policy = policy
        .get("publication")
        .and_then(Value::as_object)
        .context("factory policy does not authorize pull-request publication")?;
    if publication_policy.get("allowed").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(
            "factory policy does not authorize pull-request publication"
        ));
    }
    let target_allowlist = publication_policy
        .get("repository_allowlist")
        .and_then(Value::as_array)
        .context("publication policy omitted repository_allowlist")?;
    if !target_allowlist
        .iter()
        .filter_map(Value::as_str)
        .any(|repository| repository.eq_ignore_ascii_case(request.target_repository))
    {
        return Err(anyhow!(
            "publication target repository is outside the factory policy allowlist"
        ));
    }
    let expected_repository = format!(
        "{}/{}",
        work_item.source_repository_owner, work_item.source_repository_name
    );
    if request.target_repository != expected_repository {
        return Err(anyhow!(
            "publication target repository must match the claimed source repository"
        ));
    }
    let expected_base_ref = publication_policy
        .get("base_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.len() <= 240)
        .context("publication policy omitted base_ref")?
        .to_owned();
    validate_factory_publication_base_ref(&expected_base_ref)?;
    if request.base_ref != expected_base_ref {
        return Err(anyhow!(
            "publication base ref {} does not match factory policy {}",
            request.base_ref,
            expected_base_ref
        ));
    }
    let expected_base_commit =
        factory_policy_required_string(policy, "source_base_commit", 64)?.to_ascii_lowercase();
    validate_factory_base_commit(&expected_base_commit)?;
    let branch_prefix = publication_policy
        .get("branch_prefix")
        .and_then(Value::as_str)
        .unwrap_or("ecorp/");
    if !request.branch.starts_with(branch_prefix) {
        return Err(anyhow!(
            "publication branch must start with the authorized prefix {branch_prefix}"
        ));
    }
    let review_status = publication_policy
        .get("review_status")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("publication policy omitted review_status")?
        .to_owned();
    let project_status_before = publication_policy
        .get("status_before")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("publication policy omitted status_before")?
        .to_owned();
    if !request.body.contains(&work_item.source_issue_url) {
        return Err(anyhow!(
            "pull request body must link the claimed source issue URL"
        ));
    }

    let row = sqlx::query(
        r#"
        SELECT deliverable.artifact_id, deliverable.task_id, deliverable.run_id,
               deliverable.form, deliverable.verification_sha256,
               deliverable.base_commit, deliverable.head_commit, deliverable.branch,
               deliverable.integration_state,
               artifact.status AS artifact_status, artifact.sha256 AS deliverable_sha256,
               artifact.metadata,
               run.status AS run_status, run.verification_status AS run_verification_status,
               run.workspace_disposition,
               run.verification_sha256 AS run_verification_sha256,
               run.deliverable_sha256 AS run_deliverable_sha256,
               run.breaker_stage,
               task.status AS task_status,
               task.verification_status AS task_verification_status,
               mission.status AS mission_status, mission.room_id
        FROM source_deliverables deliverable
        JOIN artifacts artifact
          ON artifact.id = deliverable.artifact_id
         AND artifact.corp_id = deliverable.corp_id
        JOIN runs run
          ON run.id = deliverable.run_id
         AND run.corp_id = deliverable.corp_id
        JOIN tasks task
          ON task.id = deliverable.task_id
         AND task.corp_id = deliverable.corp_id
        JOIN missions mission
          ON mission.id = task.mission_id
         AND mission.corp_id = deliverable.corp_id
        WHERE deliverable.id = $1
          AND deliverable.corp_id = $2
          AND mission.id = $3
        FOR UPDATE OF deliverable, artifact, run, task, mission
        "#,
    )
    .bind(request.source_deliverable_id)
    .bind(request.corp_id)
    .bind(mission_id)
    .fetch_optional(&mut **tx)
    .await?
    .context("source deliverable is not linked to the factory mission")?;
    let form: String = row.get("form");
    let head_commit: Option<String> = row.get("head_commit");
    let commit_sha = head_commit
        .filter(|value| !value.is_empty())
        .context("merge-ready publication requires a committed deliverable")?
        .to_ascii_lowercase();
    validate_factory_base_commit(&commit_sha)?;
    let base_commit: String = row.get::<String, _>("base_commit").to_ascii_lowercase();
    if form != "commit_branch"
        || row.get::<String, _>("integration_state") != "ready_for_review"
        || base_commit != expected_base_commit
        || commit_sha == base_commit
    {
        return Err(anyhow!(
            "source deliverable is not a merge-ready commit/branch result"
        ));
    }
    let artifact_metadata: Value = row.get("metadata");
    if artifact_metadata
        .get("publication_ready")
        .and_then(Value::as_bool)
        != Some(true)
        || artifact_metadata
            .get("git_bundle_sha256")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(anyhow!(
            "source deliverable does not include a verified portable Git bundle"
        ));
    }
    let deliverable_sha256: String = row.get("deliverable_sha256");
    let verification_sha256: String = row.get("verification_sha256");
    if row.get::<String, _>("artifact_status") != "ready"
        || row.get::<String, _>("run_status") != "completed"
        || row.get::<String, _>("run_verification_status") != "passed"
        || row
            .get::<Option<String>, _>("workspace_disposition")
            .as_deref()
            == Some("quarantined")
        || row.get::<String, _>("task_status") != "completed"
        || row.get::<String, _>("task_verification_status") != "passed"
        || row.get::<String, _>("mission_status") != "completed"
        || row
            .get::<Option<String>, _>("run_verification_sha256")
            .as_deref()
            != Some(verification_sha256.as_str())
        || row
            .get::<Option<String>, _>("run_deliverable_sha256")
            .as_deref()
            != Some(deliverable_sha256.as_str())
    {
        return Err(anyhow!(
            "source deliverable is not linked to passing persisted verifier state"
        ));
    }
    let selected_run_id: Uuid = row.get("run_id");
    ensure_run_not_hard_blocked_tx(
        tx,
        request.corp_id,
        selected_run_id,
        row.get::<String, _>("breaker_stage").as_str(),
        "pull-request publication",
    )
    .await?;

    let task_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM tasks WHERE corp_id = $1 AND mission_id = $2 ORDER BY created_at, id",
    )
    .bind(request.corp_id)
    .bind(mission_id)
    .fetch_all(&mut **tx)
    .await?;
    let run_rows = sqlx::query(
        r#"
        SELECT run.id, run.resumed_from_run_id, run.breaker_stage, run.workspace_disposition,
               run.no_progress_events, run.repeated_tool_count,
               COALESCE(policy.no_progress_event_limit, 8) AS no_progress_limit,
               COALESCE(policy.repeated_tool_limit, 5) AS repeated_tool_limit
        FROM runs run
        JOIN tasks task ON task.id = run.task_id
        LEFT JOIN corp_budget_policies policy ON policy.corp_id = run.corp_id
        WHERE run.corp_id = $1 AND task.mission_id = $2
        ORDER BY run.created_at, run.id
        FOR UPDATE OF run
        "#,
    )
    .bind(request.corp_id)
    .bind(mission_id)
    .fetch_all(&mut **tx)
    .await?;
    let resume_edges = run_rows
        .iter()
        .map(|run| {
            (
                run.get::<Uuid, _>("id"),
                run.get::<Option<Uuid>, _>("resumed_from_run_id"),
            )
        })
        .collect::<Vec<_>>();
    let selected_lineage = publication_resume_lineage(selected_run_id, &resume_edges)?;
    let checkpoint = checkpoint_publication::authority_tx(
        tx,
        request.corp_id,
        work_item.id,
        selected_run_id,
        &selected_lineage,
        &commit_sha,
    )
    .await?;
    if run_rows.iter().any(|run| {
        selected_lineage.contains(&run.get::<Uuid, _>("id"))
            && run
                .get::<Option<String>, _>("workspace_disposition")
                .as_deref()
                == Some("quarantined")
    }) {
        return Err(anyhow!("quarantined source lineage cannot be published"));
    }
    let completed_recoveries = sqlx::query(
        r#"
        SELECT id, observed_source_revision, replacement_run_id
        FROM factory_verification_recoveries
        WHERE corp_id = $1
          AND factory_work_item_id = $2
          AND mission_id = $3
          AND task_id = $4
          AND status = 'completed'
          AND replacement_run_id IS NOT NULL
        ORDER BY created_at DESC, id DESC
        "#,
    )
    .bind(request.corp_id)
    .bind(work_item.id)
    .bind(mission_id)
    .bind(row.get::<Uuid, _>("task_id"))
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|recovery| {
        (
            recovery.get::<Uuid, _>("id"),
            recovery.get::<String, _>("observed_source_revision"),
            recovery.get::<Uuid, _>("replacement_run_id"),
        )
    })
    .collect::<Vec<_>>();
    let completed_recovery =
        publication_recovery_for_lineage(&completed_recoveries, &selected_lineage);
    let (effective_source_revision, source_recovery_id) =
        publication_source_revision(&work_item.source_revision, completed_recovery);
    let mut run_ids = Vec::with_capacity(run_rows.len());
    for run in run_rows {
        let run_id: Uuid = run.get("id");
        let breaker_stage: String = run.get("breaker_stage");
        if run_id == selected_run_id {
            run_ids.push(run_id);
            continue;
        }
        if checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.origin_run_id == run_id)
        {
            // This exact measured budget stop produced the authorized checkpoint.
            // Publishing its reviewed bytes starts no model; loops are not waived.
            ensure_recovered_suspend_loop_metrics_allow_publication(
                run.get("no_progress_events"),
                run.get("repeated_tool_count"),
                run.get("no_progress_limit"),
                run.get("repeated_tool_limit"),
            )?;
            run_ids.push(run_id);
            continue;
        }
        if historical_suspend_is_recovered(
            run_id,
            selected_run_id,
            &breaker_stage,
            &selected_lineage,
        ) {
            ensure_recovered_suspend_loop_metrics_allow_publication(
                run.get("no_progress_events"),
                run.get("repeated_tool_count"),
                run.get("no_progress_limit"),
                run.get("repeated_tool_limit"),
            )?;
            run_ids.push(run_id);
            continue;
        }
        if checkpoint.is_some() {
            // Healthy historical tasks must not inherit a new model-budget veto
            // from the shared mission/actor counters during this zero-provider effect.
            // Their real stop/suspend state and current loop limits still apply.
            ensure_breaker_allows_human_progress(&breaker_stage, "pull-request publication")?;
            ensure_recovered_suspend_loop_metrics_allow_publication(
                run.get("no_progress_events"),
                run.get("repeated_tool_count"),
                run.get("no_progress_limit"),
                run.get("repeated_tool_limit"),
            )?;
        } else {
            ensure_run_not_hard_blocked_tx(
                tx,
                request.corp_id,
                run_id,
                &breaker_stage,
                "pull-request publication",
            )
            .await?;
        }
        run_ids.push(run_id);
    }
    let evidence_ids = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT evidence.id
        FROM verification_evidence evidence
        JOIN tasks task ON task.id = evidence.task_id
        WHERE evidence.corp_id = $1
          AND task.mission_id = $2
          AND evidence.status = 'passed'
        ORDER BY evidence.created_at, evidence.check_index, evidence.id
        "#,
    )
    .bind(request.corp_id)
    .bind(mission_id)
    .fetch_all(&mut **tx)
    .await?;
    if evidence_ids.is_empty() {
        return Err(anyhow!(
            "pull-request publication requires persisted passing verification evidence"
        ));
    }
    Ok(PublicationPrerequisites {
        work_item,
        effective_source_revision,
        source_recovery_id,
        mission_id,
        room_id: row.get("room_id"),
        artifact_id: row.get("artifact_id"),
        task_id: row.get("task_id"),
        run_id: selected_run_id,
        commit_sha,
        deliverable_sha256,
        verification_sha256,
        base_commit,
        source_branch: row.get("branch"),
        project_status_before,
        review_status,
        task_ids,
        run_ids,
        evidence_ids,
        checkpoint,
    })
}

fn publication_source_revision(
    claimed_revision: &str,
    completed_recovery: Option<(Uuid, String)>,
) -> (String, Option<Uuid>) {
    match completed_recovery {
        Some((recovery_id, observed_revision)) => (observed_revision, Some(recovery_id)),
        None => (claimed_revision.to_owned(), None),
    }
}

fn publication_recovery_for_lineage(
    completed_recoveries: &[(Uuid, String, Uuid)],
    selected_lineage: &HashSet<Uuid>,
) -> Option<(Uuid, String)> {
    completed_recoveries
        .iter()
        .find(|(_, _, replacement_run_id)| selected_lineage.contains(replacement_run_id))
        .map(|(recovery_id, observed_revision, _)| (*recovery_id, observed_revision.clone()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationSourceProvenanceState {
    Current,
    Legacy,
    Invalid,
}

fn publication_source_provenance_state(
    provenance: &Value,
    claimed_revision: &str,
    effective_revision: &str,
    recovery_id: Option<Uuid>,
) -> Result<PublicationSourceProvenanceState> {
    let schema_version = provenance.get("schema_version").and_then(Value::as_u64);
    let persisted_revision = provenance
        .pointer("/source_issue/revision")
        .and_then(Value::as_str);
    if schema_version == Some(1) {
        return Ok(if persisted_revision == Some(claimed_revision) {
            PublicationSourceProvenanceState::Legacy
        } else {
            PublicationSourceProvenanceState::Invalid
        });
    }
    if !matches!(schema_version, Some(2 | 3)) {
        return Ok(PublicationSourceProvenanceState::Invalid);
    }
    let persisted_claimed_revision = provenance
        .pointer("/source_issue/claimed_revision")
        .and_then(Value::as_str);
    let persisted_recovery = match provenance.pointer("/source_issue/recovery_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => {
            Some(Uuid::parse_str(value).context("publication provenance recovery id is invalid")?)
        }
        Some(_) => {
            return Err(anyhow!(
                "publication provenance recovery id has an invalid type"
            ));
        }
    };
    Ok(
        if persisted_revision == Some(effective_revision)
            && persisted_claimed_revision == Some(claimed_revision)
            && persisted_recovery == recovery_id
        {
            PublicationSourceProvenanceState::Current
        } else {
            PublicationSourceProvenanceState::Invalid
        },
    )
}

fn upgrade_publication_source_provenance(
    provenance: &Value,
    claimed_revision: &str,
    effective_revision: &str,
    recovery_id: Option<Uuid>,
) -> Result<Value> {
    if publication_source_provenance_state(
        provenance,
        claimed_revision,
        effective_revision,
        recovery_id,
    )? != PublicationSourceProvenanceState::Legacy
    {
        return Err(anyhow!(
            "publication provenance is not an upgradeable schema-version-1 record"
        ));
    }
    let mut upgraded = provenance.clone();
    let root = upgraded
        .as_object_mut()
        .context("publication provenance must be an object")?;
    root.insert("schema_version".to_owned(), json!(2));
    let source_issue = root
        .get_mut("source_issue")
        .and_then(Value::as_object_mut)
        .context("publication provenance omitted source issue")?;
    source_issue.insert("revision".to_owned(), json!(effective_revision));
    source_issue.insert("claimed_revision".to_owned(), json!(claimed_revision));
    source_issue.insert("recovery_id".to_owned(), json!(recovery_id));
    Ok(upgraded)
}

fn ensure_publication_matches_start(
    publication: &PullRequestPublication,
    input: &StartPullRequestPublicationInput,
) -> Result<()> {
    if publication.factory_work_item_id != input.work_item_id
        || publication.source_deliverable_id != input.source_deliverable_id
        || publication.target_repository != input.target_repository
        || publication.base_ref != input.base_ref
        || publication.branch != input.branch
        || publication.title != input.title
        || publication.body != input.body
        || publication.effect_key != input.effect_key
    {
        return Err(anyhow!(
            "conflict: publication request does not match the durable effect"
        ));
    }
    Ok(())
}

fn publication_state_rank(state: PullRequestPublicationState) -> u8 {
    match state {
        PullRequestPublicationState::Requested => 0,
        PullRequestPublicationState::Publishing => 1,
        PullRequestPublicationState::BranchPushed => 2,
        PullRequestPublicationState::PullRequestCreated => 3,
        PullRequestPublicationState::Published => 4,
    }
}

fn normalize_checkpoint(
    checkpoint: PullRequestPublicationCheckpointInput,
) -> Result<(&'static str, PullRequestPublicationCheckpointInput, Value)> {
    match checkpoint {
        PullRequestPublicationCheckpointInput::BranchPushed { commit_sha } => {
            let commit_sha = commit_sha.trim().to_ascii_lowercase();
            validate_factory_base_commit(&commit_sha)?;
            let normalized = PullRequestPublicationCheckpointInput::BranchPushed {
                commit_sha: commit_sha.clone(),
            };
            Ok((
                "branch_pushed",
                normalized,
                json!({"kind": "branch_pushed", "commit_sha": commit_sha}),
            ))
        }
        PullRequestPublicationCheckpointInput::PullRequestCreated {
            number,
            node_id,
            url,
            state,
            draft,
            title,
            body,
            head_ref,
            base_ref,
            head_sha,
            head_repository_owner,
            is_cross_repository,
            auto_merge_enabled,
        } => {
            if number <= 0 {
                return Err(anyhow!("pull request number must be positive"));
            }
            let node_id = normalize_factory_identifier(&node_id, "pull request node id", 200)?;
            let url = normalize_factory_text(&url, "pull request URL", 500)?;
            let state = normalize_factory_identifier(&state, "pull request state", 40)?
                .to_ascii_uppercase();
            let title = normalize_factory_text(&title, "pull request title", 256)?;
            let body = normalize_publication_body(&body)?;
            let head_ref = normalize_factory_identifier(&head_ref, "pull request head ref", 500)?;
            let base_ref = normalize_factory_identifier(&base_ref, "pull request base ref", 500)?;
            validate_factory_base_ref(&base_ref)?;
            let head_sha = head_sha.trim().to_ascii_lowercase();
            validate_factory_base_commit(&head_sha)?;
            let head_repository_owner = normalize_github_component(
                &head_repository_owner,
                "pull request head repository owner",
                100,
            )?;
            let normalized = PullRequestPublicationCheckpointInput::PullRequestCreated {
                number,
                node_id: node_id.clone(),
                url: url.clone(),
                state: state.clone(),
                draft,
                title: title.clone(),
                body: body.clone(),
                head_ref: head_ref.clone(),
                base_ref: base_ref.clone(),
                head_sha: head_sha.clone(),
                head_repository_owner: head_repository_owner.clone(),
                is_cross_repository,
                auto_merge_enabled,
            };
            Ok((
                "pull_request_created",
                normalized,
                json!({
                    "kind": "pull_request_created",
                    "number": number,
                    "node_id": node_id,
                    "url": url,
                    "state": state,
                    "draft": draft,
                    "title": title,
                    "body": body,
                    "head_ref": head_ref,
                    "base_ref": base_ref,
                    "head_sha": head_sha,
                    "head_repository_owner": head_repository_owner,
                    "is_cross_repository": is_cross_repository,
                    "auto_merge_enabled": auto_merge_enabled
                }),
            ))
        }
        PullRequestPublicationCheckpointInput::Published {
            project_status,
            project_field_id,
            project_option_id,
        } => {
            let project_status =
                normalize_factory_text(&project_status, "GitHub Project status", 100)?;
            let project_field_id =
                normalize_factory_identifier(&project_field_id, "GitHub Project field id", 200)?;
            let project_option_id =
                normalize_factory_identifier(&project_option_id, "GitHub Project option id", 200)?;
            let normalized = PullRequestPublicationCheckpointInput::Published {
                project_status: project_status.clone(),
                project_field_id: project_field_id.clone(),
                project_option_id: project_option_id.clone(),
            };
            Ok((
                "published",
                normalized,
                json!({
                    "kind": "published",
                    "project_status": project_status,
                    "project_field_id": project_field_id,
                    "project_option_id": project_option_id
                }),
            ))
        }
        PullRequestPublicationCheckpointInput::Failed { failure_detail } => {
            let failure_detail = normalize_publication_failure(&failure_detail)?;
            let normalized = PullRequestPublicationCheckpointInput::Failed {
                failure_detail: failure_detail.clone(),
            };
            Ok((
                "failed",
                normalized,
                json!({"kind": "failed", "failure_detail": failure_detail}),
            ))
        }
    }
}

fn normalize_publication_failure(value: &str) -> Result<String> {
    let flattened = value.split_whitespace().collect::<Vec<_>>().join(" ");
    normalize_factory_text(&flattened, "publication failure detail", 2_000)
}

#[allow(clippy::too_many_arguments)]
fn validate_pull_request_identity(
    publication: &PullRequestPublication,
    number: i64,
    _node_id: &str,
    url: &str,
    state: &str,
    title: &str,
    body: &str,
    head_ref: &str,
    base_ref: &str,
    head_sha: &str,
    head_repository_owner: &str,
    is_cross_repository: bool,
    auto_merge_enabled: bool,
) -> Result<()> {
    if auto_merge_enabled {
        return Err(anyhow!(
            "factory publication refuses pull requests with auto-merge enabled"
        ));
    }
    if state != "OPEN" {
        return Err(anyhow!(
            "factory publication requires an open pull request, not {state}"
        ));
    }
    if title != publication.title || body != publication.body {
        return Err(anyhow!(
            "pull request title or body does not match the authorized publication content"
        ));
    }
    if head_ref != publication.branch {
        return Err(anyhow!(
            "pull request head does not match the authorized publication target"
        ));
    }
    if head_ref == base_ref {
        return Err(anyhow!(
            "pull request head branch must differ from its resolved base branch"
        ));
    }
    if publication.base_ref == "HEAD" {
        if base_ref == "HEAD" || base_ref.starts_with("refs/") {
            return Err(anyhow!(
                "symbolic publication base HEAD must resolve to an explicit GitHub branch name"
            ));
        }
    } else if base_ref != publication.base_ref.trim_start_matches("refs/heads/") {
        return Err(anyhow!(
            "pull request base does not match the authorized publication target"
        ));
    }
    let target_owner = publication
        .target_repository
        .split_once('/')
        .map(|(owner, _)| owner)
        .context("publication target repository omitted owner")?;
    if is_cross_repository
        || !head_repository_owner.eq_ignore_ascii_case(target_owner)
        || !head_sha.eq_ignore_ascii_case(&publication.commit_sha)
    {
        return Err(anyhow!(
            "pull request head repository or commit does not match the verified publication target"
        ));
    }
    if !github_pull_request_url_matches(url, &publication.target_repository, number) {
        return Err(anyhow!(
            "pull request URL does not match the authorized repository and number"
        ));
    }
    Ok(())
}

fn github_pull_request_url_matches(url: &str, target_repository: &str, number: i64) -> bool {
    let Some(path) = url
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
    else {
        return false;
    };
    let mut path_parts = path.split('/');
    let Some(url_owner) = path_parts.next() else {
        return false;
    };
    let Some(url_repository) = path_parts.next() else {
        return false;
    };
    let expected_number = number.to_string();
    if path_parts.next() != Some("pull")
        || path_parts.next() != Some(expected_number.as_str())
        || path_parts.next().is_some()
    {
        return false;
    }
    let Some((target_owner, target_name)) = target_repository.split_once('/') else {
        return false;
    };
    url_owner.eq_ignore_ascii_case(target_owner) && url_repository.eq_ignore_ascii_case(target_name)
}

#[allow(clippy::too_many_arguments)]
fn ensure_pull_request_identity_matches(
    publication: &PullRequestPublication,
    number: i64,
    node_id: &str,
    url: &str,
    state: &str,
    draft: bool,
    base_ref: &str,
    head_sha: &str,
    head_repository_owner: &str,
    is_cross_repository: bool,
) -> Result<()> {
    if publication.pull_request_number != Some(number)
        || publication.pull_request_node_id.as_deref() != Some(node_id)
        || publication.pull_request_url.as_deref() != Some(url)
        || publication.pull_request_state.as_deref() != Some(state)
        || publication.pull_request_draft != Some(draft)
        || publication.pull_request_base_ref.as_deref() != Some(base_ref)
        || publication.pull_request_head_sha.as_deref() != Some(head_sha)
        || publication.pull_request_head_repository_owner.as_deref() != Some(head_repository_owner)
        || publication.pull_request_is_cross_repository != Some(is_cross_repository)
    {
        return Err(anyhow!(
            "conflict: pull request identity does not match the durable publication"
        ));
    }
    Ok(())
}

fn publication_resume_lineage(
    selected_run_id: Uuid,
    resume_edges: &[(Uuid, Option<Uuid>)],
) -> Result<HashSet<Uuid>> {
    let parents = resume_edges.iter().copied().collect::<HashMap<_, _>>();
    if parents.len() != resume_edges.len() {
        return Err(anyhow!(
            "pull-request publication run lineage contains duplicate run identifiers"
        ));
    }
    let mut lineage = HashSet::new();
    let mut current = Some(selected_run_id);
    while let Some(run_id) = current {
        if !lineage.insert(run_id) {
            return Err(anyhow!(
                "pull-request publication run lineage contains a resume cycle"
            ));
        }
        current = *parents.get(&run_id).with_context(|| {
            format!(
                "pull-request publication run lineage references run {run_id} outside the factory mission"
            )
        })?;
    }
    Ok(lineage)
}

fn historical_suspend_is_recovered(
    run_id: Uuid,
    selected_run_id: Uuid,
    breaker_stage: &str,
    selected_lineage: &HashSet<Uuid>,
) -> bool {
    run_id != selected_run_id && breaker_stage == "suspend" && selected_lineage.contains(&run_id)
}

fn ensure_recovered_suspend_loop_metrics_allow_publication(
    no_progress_events: i32,
    repeated_tool_count: i32,
    no_progress_limit: i32,
    repeated_tool_limit: i32,
) -> Result<()> {
    if (no_progress_limit > 0 && no_progress_events >= no_progress_limit)
        || (repeated_tool_limit > 0 && repeated_tool_count >= repeated_tool_limit)
    {
        return Err(anyhow!(
            "pull-request publication is blocked because current loop metrics require a hard breaker"
        ));
    }
    Ok(())
}

async fn insert_publication_attempt_tx(
    tx: &mut Transaction<'_, Postgres>,
    publication: &PullRequestPublication,
    actor_id: Uuid,
    authorization_id: Uuid,
    authorization: &Value,
    publisher_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pull_request_publication_attempts
            (id, corp_id, publication_id, attempt, actor_id, authorization_id,
             authorization_snapshot, publisher_id, state)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'running')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(publication.corp_id)
    .bind(publication.id)
    .bind(publication.attempt_count)
    .bind(actor_id)
    .bind(authorization_id)
    .bind(authorization)
    .bind(publisher_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn ensure_active_publication_control_tx(
    tx: &mut Transaction<'_, Postgres>,
    publication: &PullRequestPublication,
    current_token: Option<Uuid>,
    control: ActivePublicationControl<'_>,
) -> Result<()> {
    if publication.state == PullRequestPublicationState::Published {
        return Err(anyhow!(
            "conflict: pull-request publication is already published"
        ));
    }
    if current_token != Some(control.presented_token) {
        return Err(anyhow!(
            "conflict: stale or unauthorized publication publisher token"
        ));
    }
    if publication.publisher_id.as_deref() != Some(control.publisher_id) {
        return Err(anyhow!(
            "conflict: publication attempt belongs to another trusted publisher"
        ));
    }
    if publication.version != control.expected_version {
        return Err(anyhow!(
            "conflict: publication version is {}, not {}",
            publication.version,
            control.expected_version
        ));
    }
    if publication
        .publisher_lease_expires_at
        .is_none_or(|expiry| expiry <= control.now)
    {
        return Err(anyhow!("conflict: publication publisher lease has expired"));
    }
    let current_actor: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT actor_id
        FROM pull_request_publication_attempts
        WHERE publication_id = $1
          AND corp_id = $2
          AND attempt = $3
          AND state = 'running'
        "#,
    )
    .bind(publication.id)
    .bind(publication.corp_id)
    .bind(publication.attempt_count)
    .fetch_optional(&mut **tx)
    .await?;
    if current_actor != Some(control.actor_id) {
        return Err(anyhow!(
            "conflict: publication attempt belongs to another authorized actor"
        ));
    }
    Ok(())
}

// This transaction touches last_used_at below. Take its write lock immediately:
// concurrent FOR SHARE readers cannot both upgrade without a deadlock.
const PUBLICATION_PUBLISHER_CREDENTIAL_LOCK_SQL: &str = r#"
    SELECT id
    FROM publication_publisher_credentials
    WHERE corp_id = $1
      AND publisher_id = $2
      AND credential_hash = $3
      AND revoked_at IS NULL
      AND expires_at > now()
    FOR UPDATE
"#;

async fn revalidate_publication_publisher_credential_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    publisher_id: &str,
    credential_hash: &str,
) -> Result<()> {
    let credential_id = sqlx::query_scalar::<_, Uuid>(PUBLICATION_PUBLISHER_CREDENTIAL_LOCK_SQL)
        .bind(corp_id)
        .bind(publisher_id)
        .bind(credential_hash)
        .fetch_optional(&mut **tx)
        .await?
        .context("forbidden: publication publisher credential is no longer authorized")?;
    sqlx::query("UPDATE publication_publisher_credentials SET last_used_at = now() WHERE id = $1")
        .bind(credential_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn replayable_publication_token(
    publication: &PullRequestPublication,
    current_token: Option<Uuid>,
    operation: &PublicationOperation,
    now: chrono::DateTime<Utc>,
) -> Option<Uuid> {
    let operation_token = operation.publisher_token?;
    (publication.state != PullRequestPublicationState::Published
        && publication.version >= operation.resulting_version
        && publication
            .publisher_lease_expires_at
            .is_some_and(|expiry| expiry > now)
        && current_token == Some(operation_token))
    .then_some(operation_token)
}

fn publication_returning_columns() -> &'static str {
    r#"
        id, corp_id, factory_work_item_id, mission_id, source_deliverable_id,
        artifact_id, task_id, run_id, source_issue_number, source_issue_url,
        target_repository, base_ref, branch, commit_sha, title, body,
        actor_id, authorization_id, authorization_snapshot, effect_key, idempotency_key,
        state, version, attempt_count, publisher_id, publisher_lease_expires_at,
        failure_detail, branch_pushed_at, pull_request_number, pull_request_node_id,
        pull_request_url, pull_request_state, pull_request_draft, pull_request_base_ref,
        pull_request_head_sha,
        pull_request_head_repository_owner, pull_request_is_cross_repository, project_owner,
        project_number, project_item_id, project_status_before, project_status_after,
        project_status_updated_at, auto_merge_enabled, merge_authorized,
        deployment_authorized, provenance, created_at, updated_at
    "#
}

async fn publication_by_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    publication_id: Uuid,
    for_update: bool,
) -> Result<Option<(PullRequestPublication, Option<Uuid>)>> {
    let suffix = if for_update { " FOR UPDATE" } else { "" };
    let query = format!(
        "{PUBLICATION_SELECT} WHERE publication.id = $1 AND publication.corp_id = $2{suffix}"
    );
    publication_query_tx(tx, &query, publication_id, corp_id).await
}

async fn publication_for_work_item_viewer_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    viewer_actor_id: Uuid,
    work_item_id: Uuid,
) -> Result<Option<(PullRequestPublication, Option<Uuid>)>> {
    let query = format!(
        r#"
        {PUBLICATION_SELECT}
        JOIN missions mission ON mission.id = publication.mission_id
        JOIN room_memberships membership ON membership.room_id = mission.room_id
        WHERE publication.factory_work_item_id = $1
          AND publication.corp_id = $2
          AND membership.actor_id = $3
        "#
    );
    let row = sqlx::query(&query)
        .bind(work_item_id)
        .bind(corp_id)
        .bind(viewer_actor_id)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|row| {
        let publisher_token = row.get("publisher_token");
        Ok((map_pull_request_publication(row)?, publisher_token))
    })
    .transpose()
}

async fn publication_query_tx(
    tx: &mut Transaction<'_, Postgres>,
    query: &str,
    id: Uuid,
    corp_id: Uuid,
) -> Result<Option<(PullRequestPublication, Option<Uuid>)>> {
    let row = sqlx::query(query)
        .bind(id)
        .bind(corp_id)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|row| {
        let publisher_token = row.get("publisher_token");
        Ok((map_pull_request_publication(row)?, publisher_token))
    })
    .transpose()
}

async fn assert_publication_room_membership_tx(
    tx: &mut Transaction<'_, Postgres>,
    publication: &PullRequestPublication,
    actor_id: Uuid,
) -> Result<()> {
    let room_id = publication_room_id_tx(tx, publication.corp_id, publication.mission_id).await?;
    assert_room_membership_tx(tx, publication.corp_id, room_id, actor_id).await
}

async fn publication_collision_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    work_item_id: Uuid,
    effect_key: &str,
    target_repository: &str,
    branch: &str,
) -> Result<Option<(PullRequestPublication, Option<Uuid>)>> {
    let query = format!(
        r#"
        {PUBLICATION_SELECT}
        WHERE publication.corp_id = $1
          AND (
              publication.factory_work_item_id = $2
              OR publication.effect_key = $3
              OR (
                  publication.target_repository = $4
                  AND publication.branch = $5
              )
          )
        FOR UPDATE
        "#
    );
    let row = sqlx::query(&query)
        .bind(corp_id)
        .bind(work_item_id)
        .bind(effect_key)
        .bind(target_repository)
        .bind(branch)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|row| {
        let publisher_token = row.get("publisher_token");
        Ok((map_pull_request_publication(row)?, publisher_token))
    })
    .transpose()
}

async fn publication_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<PublicationOperation>> {
    let row = sqlx::query(
        r#"
        SELECT publication_id, actor_id, operation, resulting_version,
               publisher_token, request
        FROM pull_request_publication_operations
        WHERE corp_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(corp_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| PublicationOperation {
        publication_id: row.get("publication_id"),
        actor_id: row.get("actor_id"),
        operation: row.get("operation"),
        resulting_version: row.get("resulting_version"),
        publisher_token: row.get("publisher_token"),
        request: row.get("request"),
    }))
}

async fn record_publication_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: NewPublicationOperation<'_>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pull_request_publication_operations
            (corp_id, idempotency_key, publication_id, actor_id, operation,
             resulting_version, publisher_token, request)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(input.corp_id)
    .bind(input.idempotency_key)
    .bind(input.publication_id)
    .bind(input.actor_id)
    .bind(input.operation)
    .bind(input.resulting_version)
    .bind(input.publisher_token)
    .bind(input.request)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn ensure_publication_operation_matches(
    operation: &PublicationOperation,
    expected_operation: &str,
    actor_id: Uuid,
    publisher_token: Option<Uuid>,
    request: &Value,
) -> Result<()> {
    if operation.operation != expected_operation
        || operation.actor_id != actor_id
        || operation.request != *request
        || publisher_token.is_some() && operation.publisher_token != publisher_token
    {
        return Err(anyhow!(
            "publication idempotency key was reused for a different operation"
        ));
    }
    Ok(())
}

async fn publication_room_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    mission_id: Uuid,
) -> Result<Uuid> {
    factory_event_room_id_tx(tx, corp_id, Some(mission_id))
        .await?
        .context("publication mission omitted its room")
}

async fn publication_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    publication: &PullRequestPublication,
    actor_id: Uuid,
    event_type: &str,
    payload: Value,
) -> Result<Option<DomainEvent>> {
    let room_id = publication_room_id_tx(tx, publication.corp_id, publication.mission_id).await?;
    append_event_tx(
        tx,
        NewEvent {
            room_id: Some(room_id),
            aggregate_version: publication.version,
            correlation_id: Some(publication.mission_id),
            ..NewEvent::new(
                publication.corp_id,
                Some(actor_id),
                event_type,
                "pull_request_publication",
                publication.id,
                format!(
                    "publication:{}:{}:{}",
                    publication.id, event_type, publication.version
                ),
                payload,
            )
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_credential_touch_locks_for_update_without_widening_authentication() {
        let query = PUBLICATION_PUBLISHER_CREDENTIAL_LOCK_SQL;
        assert!(query.trim_end().ends_with("FOR UPDATE"));
        assert!(!query.contains("FOR SHARE"));
        for predicate in [
            "corp_id = $1",
            "publisher_id = $2",
            "credential_hash = $3",
            "revoked_at IS NULL",
            "expires_at > now()",
        ] {
            assert!(
                query.contains(predicate),
                "credential predicate {predicate}"
            );
        }
    }

    #[test]
    fn publication_failure_is_single_line_and_bounded() {
        let normalized =
            normalize_publication_failure("remote failed\r\nretry\tlater").expect("normalize");
        assert_eq!(normalized, "remote failed retry later");
        assert!(normalize_publication_failure(&"x".repeat(2_001)).is_err());
    }

    #[test]
    fn publication_states_are_monotonic() {
        assert!(
            publication_state_rank(PullRequestPublicationState::Publishing)
                < publication_state_rank(PullRequestPublicationState::BranchPushed)
        );
        assert!(
            publication_state_rank(PullRequestPublicationState::PullRequestCreated)
                < publication_state_rank(PullRequestPublicationState::Published)
        );
    }

    #[test]
    fn publication_source_revision_links_completed_recovery() {
        let recovery_id = Uuid::new_v4();
        assert_eq!(
            publication_source_revision(
                "claimed-revision",
                Some((recovery_id, "reviewed-revision".to_owned())),
            ),
            ("reviewed-revision".to_owned(), Some(recovery_id))
        );
        assert_eq!(
            publication_source_revision("claimed-revision", None),
            ("claimed-revision".to_owned(), None)
        );
    }

    #[test]
    fn publication_recovery_selection_follows_the_selected_resume_lineage() {
        let unrelated_run = Uuid::new_v4();
        let recovery_run = Uuid::new_v4();
        let selected_run = Uuid::new_v4();
        let unrelated_recovery = Uuid::new_v4();
        let matching_recovery = Uuid::new_v4();
        let lineage = publication_resume_lineage(
            selected_run,
            &[
                (unrelated_run, None),
                (recovery_run, None),
                (selected_run, Some(recovery_run)),
            ],
        )
        .expect("valid lineage");
        assert_eq!(
            publication_recovery_for_lineage(
                &[
                    (
                        unrelated_recovery,
                        "unrelated-revision".to_owned(),
                        unrelated_run,
                    ),
                    (
                        matching_recovery,
                        "reviewed-revision".to_owned(),
                        recovery_run,
                    ),
                ],
                &lineage,
            ),
            Some((matching_recovery, "reviewed-revision".to_owned()))
        );
    }

    #[test]
    fn publication_source_provenance_is_backward_compatible_and_versioned() {
        let recovery_id = Uuid::new_v4();
        let legacy = json!({
            "schema_version": 1,
            "source_issue": {"revision": "claimed-revision"},
        });
        assert_eq!(
            publication_source_provenance_state(
                &legacy,
                "claimed-revision",
                "reviewed-revision",
                Some(recovery_id),
            )
            .expect("legacy provenance"),
            PublicationSourceProvenanceState::Legacy
        );
        let upgraded = upgrade_publication_source_provenance(
            &legacy,
            "claimed-revision",
            "reviewed-revision",
            Some(recovery_id),
        )
        .expect("upgrade legacy provenance");
        assert_eq!(upgraded["schema_version"], 2);
        assert_eq!(upgraded["source_issue"]["revision"], "reviewed-revision");
        assert_eq!(
            upgraded["source_issue"]["claimed_revision"],
            "claimed-revision"
        );
        assert_eq!(upgraded["source_issue"]["recovery_id"], json!(recovery_id));
        let current = json!({
            "schema_version": 2,
            "source_issue": {
                "revision": "reviewed-revision",
                "claimed_revision": "claimed-revision",
                "recovery_id": recovery_id,
            },
        });
        assert_eq!(
            publication_source_provenance_state(
                &current,
                "claimed-revision",
                "reviewed-revision",
                Some(recovery_id),
            )
            .expect("current provenance"),
            PublicationSourceProvenanceState::Current
        );
        assert_eq!(
            publication_source_provenance_state(
                &json!({
                    "schema_version": 2,
                    "source_issue": {
                        "revision": "claimed-revision",
                        "claimed_revision": "claimed-revision",
                        "recovery_id": recovery_id,
                    },
                }),
                "claimed-revision",
                "reviewed-revision",
                Some(recovery_id),
            )
            .expect("mismatched provenance"),
            PublicationSourceProvenanceState::Invalid
        );
        assert_eq!(
            publication_source_provenance_state(
                &json!({"source_issue": {"revision": "claimed-revision"}}),
                "claimed-revision",
                "reviewed-revision",
                Some(recovery_id),
            )
            .expect("missing schema provenance"),
            PublicationSourceProvenanceState::Invalid
        );
    }

    #[test]
    fn publication_branch_validation_matches_git_rejections() {
        assert!(validate_factory_branch_ref("ecorp/issue-61-safe").is_ok());
        assert!(validate_factory_branch_ref("ecorp/foo//bar").is_err());
        assert!(validate_factory_branch_ref("ecorp/foo.lock").is_err());
        assert!(validate_factory_branch_ref("ecorp/foo.lock/bar").is_err());
        assert!(validate_factory_branch_ref("ecorp/.hidden").is_err());
        assert!(validate_factory_branch_ref("HEAD").is_err());
    }

    #[test]
    fn github_pull_request_urls_compare_repository_case_insensitively() {
        assert!(github_pull_request_url_matches(
            "https://github.com/Owner/MyRepo/pull/41",
            "owner/myrepo",
            41
        ));
        assert!(github_pull_request_url_matches(
            "https://github.com/OWNER/MYREPO/pull/41/",
            "owner/myrepo",
            41
        ));
        assert!(!github_pull_request_url_matches(
            "https://github.com/owner/other/pull/41",
            "owner/myrepo",
            41
        ));
        assert!(!github_pull_request_url_matches(
            "https://github.com/owner/myrepo/pull/42",
            "owner/myrepo",
            41
        ));
    }

    #[test]
    fn only_suspend_ancestors_are_treated_as_recovered_for_publication() {
        let suspended = Uuid::from_u128(1);
        let verifier_retry = Uuid::from_u128(2);
        let selected = Uuid::from_u128(3);
        let unrelated = Uuid::from_u128(4);
        let lineage = publication_resume_lineage(
            selected,
            &[
                (suspended, None),
                (verifier_retry, Some(suspended)),
                (selected, Some(verifier_retry)),
                (unrelated, None),
            ],
        )
        .expect("valid lineage");

        assert!(historical_suspend_is_recovered(
            suspended, selected, "suspend", &lineage
        ));
        assert!(!historical_suspend_is_recovered(
            unrelated, selected, "suspend", &lineage
        ));
        assert!(!historical_suspend_is_recovered(
            suspended, selected, "stop", &lineage
        ));
        assert!(!historical_suspend_is_recovered(
            selected, selected, "suspend", &lineage
        ));
        assert!(ensure_recovered_suspend_loop_metrics_allow_publication(7, 4, 8, 5).is_ok());
        assert!(ensure_recovered_suspend_loop_metrics_allow_publication(8, 4, 8, 5).is_err());
        assert!(ensure_recovered_suspend_loop_metrics_allow_publication(7, 5, 8, 5).is_err());
    }

    #[test]
    fn publication_resume_lineage_fails_closed_on_missing_or_cyclic_parents() {
        let selected = Uuid::from_u128(1);
        let missing = Uuid::from_u128(2);
        assert!(
            publication_resume_lineage(selected, &[(selected, Some(missing))])
                .unwrap_err()
                .to_string()
                .contains("outside the factory mission")
        );

        let other = Uuid::from_u128(3);
        assert!(
            publication_resume_lineage(
                selected,
                &[(selected, Some(other)), (other, Some(selected))]
            )
            .unwrap_err()
            .to_string()
            .contains("resume cycle")
        );
    }
}
