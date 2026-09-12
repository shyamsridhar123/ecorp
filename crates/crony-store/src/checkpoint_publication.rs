//! Bind zero-provider publication to the native checkpoint and reviewed export.
use super::*;

#[derive(Debug)]
pub(super) struct CheckpointPublication {
    corp_id: Uuid,
    item_id: Uuid,
    selected_run_id: Uuid,
    authority: budget_checkpoint::Authority,
    provenance: Value,
}

impl CheckpointPublication {
    pub(super) fn origin_run_id(&self) -> Uuid {
        self.authority.checkpoint.run_id
    }

    pub(super) fn provenance(&self) -> &Value {
        &self.provenance
    }

    /// This receipt is constructed only by current native validation below and
    /// consumed in the same locked publication transaction. It is not a DTO.
    pub(super) fn authority_for(
        &self,
        item: &FactoryWorkItem,
        selected_run_id: Uuid,
    ) -> Result<&budget_checkpoint::Authority> {
        if self.corp_id != item.corp_id
            || self.item_id != item.id
            || self.selected_run_id != selected_run_id
            || self.authority.checkpoint.corp_id != item.corp_id
            || Some(self.authority.checkpoint.mission_id) != item.mission_id
        {
            return Err(anyhow!(
                "publication checkpoint receipt belongs to another scope"
            ));
        }
        Ok(&self.authority)
    }
}

pub(super) async fn authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    corp_id: Uuid,
    item_id: Uuid,
    run_id: Uuid,
    selected_lineage: &HashSet<Uuid>,
    verified_head: &str,
) -> Result<Option<CheckpointPublication>> {
    let row = sqlx::query(
        r#"
        SELECT recovery.id, recovery.source_run_id, recovery.checkpoint_authority,
               run.workspace_fingerprint, run.workspace_run_id, task.mission_id
        FROM factory_verification_recoveries recovery
        JOIN runs run ON run.id=recovery.replacement_run_id AND run.corp_id=recovery.corp_id
         AND run.task_id=recovery.task_id AND run.resumed_from_run_id=recovery.source_run_id
        JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
         AND task.mission_id=recovery.mission_id
        WHERE recovery.corp_id=$1 AND recovery.factory_work_item_id=$2
          AND run.id=$3 AND recovery.mode='checkpoint_verification'
          AND recovery.status='completed' AND run.status='completed'
          AND run.verification_status='passed' AND run.workspace_disposition='preserved'
        "#,
    )
    .bind(corp_id)
    .bind(item_id)
    .bind(run_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if !budget_checkpoint::zero_provider_allocation_tx(tx, corp_id, run_id).await? {
        return Err(anyhow!(
            "checkpoint publication lost its zero-provider authority"
        ));
    }
    let authority =
        budget_checkpoint::source_authority_tx(tx, corp_id, item_id, row.get("source_run_id"))
            .await?;
    if row.get::<Option<Value>, _>("checkpoint_authority")
        != Some(serde_json::to_value(&authority)?)
        || row
            .get::<Option<String>, _>("workspace_fingerprint")
            .as_deref()
            != Some(authority.checkpoint.workspace_fingerprint.as_str())
        || row.get::<Uuid, _>("workspace_run_id") != authority.checkpoint.workspace_run_id
        || !selected_lineage.contains(&authority.checkpoint.run_id)
        || checkpoint_retention::exported_head_tx(tx, corp_id, run_id)
            .await?
            .as_deref()
            != Some(verified_head)
    {
        return Err(anyhow!(
            "checkpoint publication does not match its native source and verified export"
        ));
    }
    let explicit_stop: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
          SELECT 1 FROM events event
          JOIN runs run ON run.id=event.aggregate_id AND run.corp_id=event.corp_id
          JOIN tasks task ON task.id=run.task_id AND task.corp_id=run.corp_id
          WHERE event.corp_id=$1 AND task.mission_id=$2
            AND event.aggregate_type='run' AND event.type='run.stop_requested'
        )
        "#,
    )
    .bind(corp_id)
    .bind(row.get::<Uuid, _>("mission_id"))
    .fetch_one(&mut **tx)
    .await?;
    if explicit_stop {
        return Err(anyhow!(
            "checkpoint publication cannot override an explicit stop"
        ));
    }
    Ok(Some(CheckpointPublication {
        corp_id,
        item_id,
        selected_run_id: run_id,
        provenance: json!({
            "schema_version":1,
            "recovery_id":row.get::<Uuid, _>("id"),
            "origin_run_id":authority.checkpoint.run_id,
            "workspace_run_id":authority.checkpoint.workspace_run_id,
            "checkpoint_event_id":authority.checkpoint_event_id,
            "termination_event_id":authority.termination_event_id,
            "workspace_fingerprint":authority.checkpoint.workspace_fingerprint,
            "source_base_commit":authority.checkpoint.source_base_commit,
            "original_head_commit":authority.checkpoint.head_commit,
            "verified_head_commit":verified_head,
            "authority_sha256":hex::encode(Sha256::digest(serde_json::to_vec(&authority)?)),
            "execution_mode":"verification_only",
        }),
        authority,
    }))
}

pub(super) fn provenance_matches(persisted: &Value, expected: Option<&Value>) -> bool {
    match expected {
        Some(expected) => persisted.get("checkpoint") == Some(expected),
        None => persisted.get("checkpoint").is_none_or(Value::is_null),
    }
}

/// The caller must first revalidate the live native authority and all publication
/// bindings. Schema 3 makes checkpoint proof mandatory; only older records may
/// acquire previously absent proof, and non-null conflicting proof never upgrades.
pub(super) fn revalidated_provenance(persisted: &Value, expected: Option<&Value>) -> Result<Value> {
    let schema = persisted.get("schema_version").and_then(Value::as_u64);
    let legacy = matches!(schema, Some(1 | 2));
    if !legacy && schema != Some(3) {
        return Err(anyhow!("unsupported publication provenance schema"));
    }
    let mut upgraded = persisted.clone();
    if let Some(expected) = expected {
        match persisted.get("checkpoint") {
            Some(proof) if !proof.is_null() && proof != expected => {
                return Err(anyhow!("publication checkpoint provenance changed"));
            }
            None | Some(Value::Null) if !legacy => {
                return Err(anyhow!("publication checkpoint provenance is missing"));
            }
            _ => {}
        }
        if legacy {
            upgraded["checkpoint"] = expected.clone();
            upgraded["schema_version"] = json!(3);
        }
    } else if schema == Some(3) {
        return Err(anyhow!("checkpoint publication lost its native authority"));
    }
    if !provenance_matches(&upgraded, expected) {
        return Err(anyhow!(
            "publication authority no longer matches its checkpoint provenance"
        ));
    }
    Ok(upgraded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_publication_provenance_cannot_be_missing_changed_or_forged() {
        let expected = json!({"origin_run_id":"source","authority_sha256":"exact"});
        assert!(provenance_matches(
            &json!({"checkpoint":expected}),
            Some(&expected)
        ));
        assert!(!provenance_matches(&json!({}), Some(&expected)));
        assert!(!provenance_matches(
            &json!({"checkpoint":null}),
            Some(&expected)
        ));
        assert!(!provenance_matches(
            &json!({"checkpoint":{"origin_run_id":"another"}}),
            Some(&expected)
        ));
        assert!(!provenance_matches(&json!({"checkpoint":expected}), None));
        assert!(provenance_matches(&json!({}), None));
        assert!(provenance_matches(&json!({"checkpoint":null}), None));
    }

    #[test]
    fn legacy_checkpoint_provenance_upgrades_without_weakening_current_proof() {
        let proof = json!({"origin_run_id":"source","authority_sha256":"exact"});
        for schema in [1, 2] {
            for checkpoint in [None, Some(Value::Null), Some(proof.clone())] {
                let mut legacy =
                    json!({"schema_version":schema,"source_issue":{"revision":"kept"}});
                if let Some(checkpoint) = checkpoint {
                    legacy["checkpoint"] = checkpoint;
                }
                let upgraded = revalidated_provenance(&legacy, Some(&proof)).unwrap();
                assert_eq!(upgraded["schema_version"], 3);
                assert_eq!(upgraded["checkpoint"], proof);
                assert_eq!(upgraded["source_issue"], legacy["source_issue"]);
                assert_eq!(
                    revalidated_provenance(&upgraded, Some(&proof)).unwrap(),
                    upgraded
                );
            }
        }
        for schema in [1, 2, 3] {
            assert!(
                revalidated_provenance(
                    &json!({"schema_version":schema,"checkpoint":{"origin_run_id":"changed"}}),
                    Some(&proof)
                )
                .is_err()
            );
        }
        for current in [
            json!({"schema_version":3}),
            json!({"schema_version":3,"checkpoint":null}),
            json!({"schema_version":4,"checkpoint":proof}),
        ] {
            assert!(revalidated_provenance(&current, Some(&proof)).is_err());
        }
        assert!(revalidated_provenance(&json!({"schema_version":3}), None).is_err());
        let ordinary = json!({"schema_version":2,"checkpoint":null});
        assert_eq!(revalidated_provenance(&ordinary, None).unwrap(), ordinary);
    }
}
