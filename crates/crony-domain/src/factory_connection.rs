//! The saved execution environment is immutable Factory policy, not a hint
//! inferred from whichever runner or account happens to be available.
use serde_json::Value;
use uuid::Uuid;

pub fn factory_workspace_connection_id(policy: &Value) -> Result<Option<Uuid>, String> {
    let policy = policy
        .as_object()
        .ok_or("factory policy snapshot must be a JSON object")?;
    match policy.get("workspace_connection_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Uuid::parse_str(value)
            .ok()
            .filter(|id| !id.is_nil())
            .map(Some)
            .ok_or_else(|| "factory workspace_connection_id must be a non-nil UUID".to_owned()),
        Some(_) => Err("factory workspace_connection_id must be a UUID or null".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn issue204_factory_binding_preserves_legacy_unbound_policy() {
        assert_eq!(factory_workspace_connection_id(&json!({})), Ok(None));
        assert_eq!(
            factory_workspace_connection_id(&json!({"workspace_connection_id": null})),
            Ok(None)
        );
    }

    #[test]
    fn issue204_factory_binding_retains_the_exact_connection() {
        let id = Uuid::new_v4();
        assert_eq!(
            factory_workspace_connection_id(&json!({"workspace_connection_id": id})),
            Ok(Some(id))
        );
    }

    #[test]
    fn issue204_factory_binding_rejects_malformed_and_nil_values_without_echoing_input() {
        for value in [
            json!(""),
            json!("private-account-diagnostics-must-not-be-echoed"),
            json!(Uuid::nil()),
            json!(42),
            json!(false),
            json!([]),
            json!({}),
        ] {
            let error = factory_workspace_connection_id(&json!({"workspace_connection_id": value}))
                .unwrap_err();
            assert!(error.starts_with("factory workspace_connection_id"));
            assert!(!error.contains("private-account"));
        }
        for invalid_policy in [Value::Null, json!([]), json!("not an object")] {
            assert!(factory_workspace_connection_id(&invalid_policy).is_err());
        }
    }
}
