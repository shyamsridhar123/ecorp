# ECorp handbook

**Run it together. Build in parallel. Proof before done.**

Start with the [interactive front office](https://ecorp-front-office.shyam-sridhar16.chatgpt.site)
for the vision and guided product tour, or [run ECorp locally](../README.md#start-locally) to operate
real missions. The public tour is illustrative; it is not connected to your runners or repositories.

## Choose your next step

| I want to… | Start here |
| --- | --- |
| Run my first mission | [User and developer journey](USER_AND_DEVELOPER_JOURNEY.md) |
| Understand the product and its direction | [Product and technical plan](PRODUCT_AND_TECHNICAL_PLAN.md) |
| Understand where state and execution live | [Architecture](ARCHITECTURE.md) |
| Review the trust boundary | [Security](SECURITY.md) and [threat model](THREAT_MODEL.md) |
| Work on ECorp safely | [Contributing](../CONTRIBUTING.md) and [contributor contract](../AGENTS.md) |
| Operate GitHub issue intake, recovery, and publication | [Dark-factory contributor guide](DARK_FACTORY_CONTRIBUTOR_GUIDE.md) |
| Change the console or desktop shell | [Web client](../apps/web/README.md) and [desktop shell](../apps/desktop/README.md) |
| Understand what counts as proof | [Evaluation and real-world testing](EVALS.md) |
| Reuse the hero, voice, and public-site link | [Brand and product site](BRAND_AND_PRODUCT_SITE.md) |
| Read design decisions | [Architecture decision records](adr/) |
| Review naming and provenance boundaries | [Brand-name and licensing review](BRAND_NAME_AND_LICENSING_REVIEW.md), [NOTICE](../NOTICE), and [LICENSE](../LICENSE) |

## Current implementation highlights

- **Mission-owned studios:** three Copilot specialists produce verified handoffs before integration.
- **Explicit launch:** a saved plan stays held until an authorized dispatch, even across restart.
- **Governed execution:** immutable source selection, isolated task worktrees, human decisions,
  budget limits, and evidence-gated completion.
- **Portable results:** signed source deliverables and separately authorized, recoverable PR
  publication. Publication does not imply merge or deployment.

These are alpha capabilities, not a claim of safe execution for fully untrusted processes.
Read the security guide before using real workloads.

## Read the evidence at its actual scope

| Topic | Dated record |
| --- | --- |
| Mission staffing, real Copilot studio, handoffs, and review limits | [September 6 studio report](evidence/2026-09-06-mission-staffing.md) |
| Held plans and explicit launch | [September 6 launch-admission report](evidence/2026-09-06-mission-launch-admission.md) |
| Copilot native filesystem and remaining negative-case coverage | [September 6 filesystem report](evidence/2026-09-06-copilot-native-filesystem.md) |
| Copilot runtime pinning and byte-exact readback | [September 6 runtime report](evidence/2026-09-06-copilot-native-read-runtime.md) |
| Factory cockpit reconnect and publication recovery | [September 4 cockpit report](evidence/2026-09-04-factory-cockpit-restart.md) |
| Windows external-provider teardown | [September 3 lifecycle report](evidence/2026-09-03-external-provider-fail-closed.md) |
| Portable source exports and authorized publication | [Deliverables](evidence/2026-09-02-portable-deliverables.md) and [publication](evidence/2026-09-02-idempotent-pull-request-publication.md) |

The [complete evidence archive](evidence/) records observed runs, including limitations and failed
attempts. A dated result is not a fresh validation of a later commit. Deterministic fixtures, real
provider execution, browser checks, human authentication, and external effects are different
evidence scopes.

## Planning without competing backlogs

[ECorp Build, GitHub Project #3](https://github.com/users/shyamsridhar123/projects/3) and its linked
issues are the operational source for priorities, ownership, dependencies, and status.
[BACKLOG.md](BACKLOG.md) is historical seed material. The product plan preserves original design
intent alongside dated implementation checkpoints; future-tense sections are not shipped-feature
claims.
