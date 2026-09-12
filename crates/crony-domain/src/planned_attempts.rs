//! ECorp task-attempt policy is chosen before execution. This is not a
//! replacement for a provider's internal retries or session lifecycle.
use serde_json::Value;

/// The existing planner ceiling, shared with public policy validation.
pub const MAX_TASK_ATTEMPTS: i32 = 3;

/// Read an optional, immutable Factory planning choice without changing the
/// legacy policy representation. Absence does not grant additional attempts.
pub fn factory_max_task_attempts(policy: &Value) -> Result<Option<i32>, String> {
    let policy = policy
        .as_object()
        .ok_or("factory policy snapshot must be a JSON object")?;
    match policy.get("max_task_attempts") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|value| (1..=i64::from(MAX_TASK_ATTEMPTS)).contains(value))
            .map(|value| Some(value as i32))
            .ok_or_else(|| {
                format!(
                    "factory max_task_attempts must be an integer between 1 and {MAX_TASK_ATTEMPTS}"
                )
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn issue224_omitted_attempt_policy_preserves_legacy_shape() {
        for policy in [json!({}), json!({"max_task_attempts": null})] {
            let before = policy.clone();
            assert_eq!(factory_max_task_attempts(&policy), Ok(None));
            assert_eq!(policy, before);
        }
    }

    #[test]
    fn issue224_explicit_attempt_policy_uses_the_existing_ceiling() {
        for value in 1..=MAX_TASK_ATTEMPTS {
            assert_eq!(
                factory_max_task_attempts(&json!({"max_task_attempts": value})),
                Ok(Some(value))
            );
        }
    }

    #[test]
    fn issue224_invalid_attempt_policy_is_rejected_without_echoing_input() {
        for value in [
            json!(0),
            json!(-1),
            json!(MAX_TASK_ATTEMPTS + 1),
            json!(i64::MAX),
            json!(3.0),
            json!("3"),
            json!("private-diagnostics-must-not-be-echoed"),
            json!(false),
            json!([]),
            json!({}),
        ] {
            let error =
                factory_max_task_attempts(&json!({"max_task_attempts": value})).unwrap_err();
            assert_eq!(
                error,
                "factory max_task_attempts must be an integer between 1 and 3"
            );
        }
        for invalid_policy in [Value::Null, json!([]), json!("not an object")] {
            assert!(factory_max_task_attempts(&invalid_policy).is_err());
        }
    }
}
