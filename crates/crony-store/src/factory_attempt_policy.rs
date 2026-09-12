//! Prospective task-attempt authority. No existing task counter or allowance
//! can be changed through materialization, replay, or a later recovery.
use anyhow::{Result, anyhow};
use crony_domain::{MAX_TASK_ATTEMPTS, TaskGraphPlan, factory_max_task_attempts};
use serde_json::Value;

pub(super) fn validate_request(policy: &Value, request: &Value) -> Result<()> {
    let allowed = factory_max_task_attempts(policy).map_err(anyhow::Error::msg)?;
    let requested = factory_max_task_attempts(request).map_err(anyhow::Error::msg)?;
    if allowed != requested {
        return Err(anyhow!(
            "factory max_task_attempts must match the immutable claimed planning policy"
        ));
    }
    Ok(())
}

pub(super) fn validate_plan(policy: &Value, plan: &TaskGraphPlan) -> Result<()> {
    let allowed = factory_max_task_attempts(policy).map_err(anyhow::Error::msg)?;
    // The stock public planners previously granted at most two attempts.
    // An old claim with no explicit allowance must not newly grant a third.
    let ceiling = allowed.unwrap_or(2);
    for task in &plan.tasks {
        if !(1..=MAX_TASK_ATTEMPTS).contains(&task.max_attempts)
            || task.max_attempts > ceiling
            || allowed.is_some_and(|expected| task.max_attempts != expected)
        {
            return Err(anyhow!(
                "factory task {} must preserve the claimed task-attempt allowance",
                task.key
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn issue224_factory_request_retains_legacy_omission_and_exact_explicit_policy() {
        assert!(validate_request(&json!({}), &json!({})).is_ok());
        assert!(validate_request(&json!({"max_task_attempts": null}), &json!({})).is_ok());
        for value in 1..=MAX_TASK_ATTEMPTS {
            assert!(
                validate_request(
                    &json!({"max_task_attempts": value}),
                    &json!({"max_task_attempts": value}),
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn issue224_factory_request_cannot_substitute_omit_or_add_attempt_authority() {
        for (policy, request) in [
            (json!({}), json!({"max_task_attempts": 3})),
            (json!({"max_task_attempts": 3}), json!({})),
            (
                json!({"max_task_attempts": 3}),
                json!({"max_task_attempts": 2}),
            ),
            (
                json!({"max_task_attempts": 2}),
                json!({"max_task_attempts": 3}),
            ),
            (
                json!({"max_task_attempts": 4}),
                json!({"max_task_attempts": 4}),
            ),
        ] {
            let before = (policy.clone(), request.clone());
            assert!(validate_request(&policy, &request).is_err());
            assert_eq!((policy, request), before);
        }
    }
}
