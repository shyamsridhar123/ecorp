//! Read-only staffing proposals. The store persists only identities used by an
//! accepted mission graph, in the same transaction as its tasks.

use anyhow::{Result, anyhow};
use chrono::Utc;
use crony_domain::{Agent, AgentStatus, PlannedAgent, TaskGraphPlan};
use uuid::Uuid;

pub fn candidates(
    corp_id: Uuid,
    strategy: &str,
    adapter: &str,
    reusable: &[Agent],
) -> Result<(Vec<Agent>, Vec<PlannedAgent>)> {
    let roles: &[(&str, &str, &str)] = match strategy {
        "single" => &[("engineer", "Delivery engineer", "cobalt")],
        "parallel-specialists" => &[
            ("specialist-a", "Research specialist", "cobalt"),
            ("specialist-b", "Implementation specialist", "mint"),
            ("manager", "Integration lead", "marigold"),
        ],
        "studio-swarm" if adapter == "github-copilot" => &[
            ("visual-direction", "Visual designer", "violet"),
            ("gameplay-systems", "Gameplay engineer", "cobalt"),
            ("quality-verification", "Quality engineer", "mint"),
        ],
        "studio-swarm" => return Err(anyhow!("studio-swarm requires GitHub Copilot")),
        _ => return Err(anyhow!("strategy does not support mission-scoped staffing")),
    };
    let mut agents = Vec::with_capacity(roles.len());
    let mut proposed = Vec::with_capacity(roles.len());
    for &(role, name, accent) in roles {
        let existing = reusable.iter().find(|agent| {
            agent.corp_id == corp_id
                && agent.pinned
                && agent.mission_id.is_some()
                && agent.retired_at.is_none()
                && agent.status == AgentStatus::Idle
                && agent.current_run_id.is_none()
                && agent.adapter == adapter
                && agent.role == role
                && !agents.iter().any(|used: &Agent| used.id == agent.id)
        });
        if let Some(agent) = existing {
            agents.push(agent.clone());
            continue;
        }
        let id = Uuid::new_v4();
        proposed.push(PlannedAgent {
            id,
            name: name.to_owned(),
            role: role.to_owned(),
            adapter: adapter.to_owned(),
            accent: accent.to_owned(),
        });
        agents.push(Agent {
            id,
            corp_id,
            actor_id: Uuid::nil(),
            name: name.to_owned(),
            role: role.to_owned(),
            adapter: adapter.to_owned(),
            status: AgentStatus::Idle,
            station: None,
            current_run_id: None,
            accent: accent.to_owned(),
            created_at: Utc::now(),
            mission_id: None,
            pinned: false,
            retired_at: None,
        });
    }
    Ok((agents, proposed))
}

pub fn attach_used_identities(plan: &mut TaskGraphPlan, proposed: Vec<PlannedAgent>) {
    plan.staffing = proposed
        .into_iter()
        .filter(|agent| {
            plan.tasks
                .iter()
                .any(|task| task.assigned_agent_id == agent.id)
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_corp_gets_only_the_required_roles() {
        let corp = Uuid::new_v4();
        for (strategy, count) in [
            ("single", 1),
            ("parallel-specialists", 3),
            ("studio-swarm", 3),
        ] {
            let (agents, proposed) = candidates(corp, strategy, "github-copilot", &[]).unwrap();
            assert_eq!(agents.len(), count);
            assert_eq!(proposed.len(), count);
            assert!(agents.iter().all(|agent| agent.corp_id == corp));
            assert_eq!(
                agents
                    .iter()
                    .map(|agent| agent.id)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                count
            );
        }
    }

    #[test]
    fn only_idle_pinned_same_corp_role_is_reused() {
        let corp = Uuid::new_v4();
        let (mut agents, _) = candidates(corp, "single", "github-copilot", &[]).unwrap();
        let original = agents[0].id;
        assert_eq!(
            candidates(corp, "single", "github-copilot", &agents)
                .unwrap()
                .1
                .len(),
            1
        );
        agents[0].pinned = true;
        agents[0].mission_id = Some(Uuid::new_v4());
        let (reused, proposed) = candidates(corp, "single", "github-copilot", &agents).unwrap();
        assert_eq!(reused[0].id, original);
        assert!(proposed.is_empty());
        agents[0].retired_at = Some(Utc::now());
        assert!(
            !candidates(corp, "single", "github-copilot", &agents)
                .unwrap()
                .1
                .is_empty()
        );
        agents[0].retired_at = None;
        agents[0].corp_id = Uuid::new_v4();
        assert!(
            !candidates(corp, "single", "github-copilot", &agents)
                .unwrap()
                .1
                .is_empty()
        );
        agents[0].corp_id = corp;
        agents[0].current_run_id = Some(Uuid::new_v4());
        assert!(
            !candidates(corp, "single", "github-copilot", &agents)
                .unwrap()
                .1
                .is_empty()
        );
    }

    #[test]
    fn studio_never_falls_back_to_another_provider() {
        assert!(candidates(Uuid::new_v4(), "studio-swarm", "codex", &[]).is_err());
    }
}
