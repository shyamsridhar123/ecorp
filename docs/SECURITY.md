# Security

## Current status

The current implementation has production authentication, workload identity, and scoped secret
boundaries plus durable artifact storage, but it is not yet suitable for fully untrusted child
processes until stronger OS/container isolation lands.

Known development-only shortcuts:

- fixed demo identities
- permissive CORS
- fake process runs with the local user's permissions
- no network sandbox
- external CLI adapters do not yet have a complete durable provider-permission bridge or verified
  descendant-process cleanup

These are explicit backlog items, not production claims.

Claude Code is launched in safe mode without user plugins, hooks, MCP servers, browser integration,
slash commands, or auto-memory. OpenCode uses its plugin-free `--pure` mode. These controls prevent
the customization leakage observed during the September 1, 2026 enterprise dogfood pass, but they
are only a partial boundary. GitHub issue #51 tracks isolated provider homes,
inherited-environment allowlisting, durable permission translation, stop deadlines, and
process-tree verification.

Production mode validates OIDC bearer tokens against the configured issuer's UserInfo endpoint and
maps `(issuer, subject)` to a Corp-local human actor. Claimed actor IDs cannot override that mapping.
Development mode still enforces room membership in persistence, snapshots, writes, WebSocket
replay, and live delivery. Eve is a deliberate non-member fixture used to prove that room-scoped
missions, tasks, runs, messages, and events are not returned.

Runner nodes require one-time enrollment followed by rotating, expiring workload credentials.
Only credential hashes are stored. Replayed, expired, unknown, and revoked credentials are denied.

Secrets are encrypted with ChaCha20-Poly1305 and authenticated associated data. The broker checks
actor, task, run, runner, tool, resource, and expiry scope before dispatch. Events and snapshots
contain grant metadata only. Environment injection is labeled reduced assurance.
Expired grants are rejected before provider start, and the provider is stopped when the earliest
active grant expires.

Risky action approvals are Corp-scoped, role-gated, expiring, and idempotent. Approval decisions
transactionally enqueue durable runner commands. Commands remain pending until the runner
acknowledges application, command IDs fence duplicate delivery, and expiry automatically rejects
the action and repairs run/task/mission/agent state.

Dark-factory claims use a separate opaque fencing token plus a monotonic work-item version.
Claim tokens are returned only to the authorized operator and are omitted from shared snapshots,
events, prompts, logs, and artifacts. A GitHub label or Project status is never treated as an
execution lock. Mission materialization requires an active lease and atomically persists the
issue-to-mission link. The trusted CLI invokes GitHub CLI without putting its credential in an
argument, log, mission contract, or agent environment.
Expired reclaims must exactly match the original source and policy snapshots and cannot widen
permissions or replace the claimed revision before materialization.
GitHub owner and repository names are canonicalized before advisory locking and database lookup so
case variants cannot bypass the one-work-item-per-Project-item boundary.

The persisted factory policy is enforced again during mission materialization. A later request
cannot widen its repository, adapter, strategy, model, reasoning effort, tools, secrets, required
prohibitions, write scope, token budget, or cost budget. Factory `verified` state requires
authoritative completed mission and passing task-verification records.
Non-null policy model and reasoning settings are mandatory on every materialized task; omission is
rejected rather than interpreted as permission to use a provider default.
Provider-backed factory tasks require a manual verification gate; an artifact-only result cannot
become accepted completion without an authorized evidence decision.
Verifier rejection is persisted as `verification_failed`, not collapsed into an execution failure.
Publication states cannot be asserted through the generic factory transition endpoint.

Guests and spectators do not receive factory work items in snapshots. Pre-materialization factory
events omit GitHub source metadata; once a mission exists, factory events inherit its room
visibility.

Controllers renew their fenced lease immediately before a GitHub mutation and again before launch.
After each renewal they re-fetch and compare the Project item, issue revision, state, required
label, and dependency eligibility. A changed or newly blocked source is durably blocked before the
next effect. A final renewal follows each revalidation, and GitHub CLI subprocesses are killed on a
bounded timeout below the effect lease. The controller resolves the source ref in a checkout whose
GitHub remote matches the claimed repository, then persists the full 40- or 64-hex commit in policy.
Factory tasks and run launch records carry repository, ref, and commit as one all-or-nothing source
identity. The scheduler accepts only a runner advertising the same structured tuple, and the runner
independently rejects both start and resume commands before worktree access if any element differs.
Symbolic refs alone are never sufficient authority for factory write-capable routing. Unpinned
ordinary tasks continue to resolve the configured ref for each new worktree, while resume is fenced
to the source run's persisted workspace base commit.

Pre-commit factory records are migrated only from unambiguous persisted workspace evidence.
Underivable legacy claims are not guessed or silently widened: migration marks them as requiring an
upgrade, and only the active fenced operator may resolve and persist a new commit before any run
exists. That dedicated idempotent operation is stored in `factory_operations` and emits
`factory.source_commit_pinned`; it rejects stale tokens, stale versions, already-pinned policies,
incompatible task contracts, and any mission that has already produced a run.
External CLI failure details are collapsed to bounded single-line text before persistence so
multi-line stderr cannot bypass the durable blocked transition.

The GitHub Copilot permission handler automatically approves writes inside the assigned worktree,
read-only operations it can prove are scoped to that worktree, and reads from the SDK state
directory isolated to that worktree. It canonicalizes existing ancestors to reject symlink escapes.
External paths, network URLs, sandbox bypass, managed-policy approvals, and ambiguous shell
commands suspend durably. Shell approval cards include the bounded command text rather than only a
generic action label.

Budget policies constrain run, mission, requester, and Corp usage. Repeated tools and explicit
no-progress events feed an auditable circuit breaker; ordinary human conversation does not.

Artifact uploads are server-mediated and size-bounded. The server verifies byte count, digest, and
declared media type in memory, then performs authoritative assignment, task, budget, and breaker
checks while reserving the event-specific staging key in Postgres. No object bytes are written
before that reservation commits. Finalization is idempotent, transient failures remain recoverable,
old missing-object reservations are released for retry, and rejected or orphaned staging cleanup
rechecks ownership before deletion and never deletes shared content-addressed bytes. The server
signs provenance with a deployment key and never exposes backend bucket URLs. Downloads require
`ready` metadata, Corp authorization, and room membership, then revalidate signature, retention,
digest, length, and media type before returning an attachment with content sniffing disabled.

Portable source exports use a temporary Git index rooted in the assigned worktree. They include
tracked changes and non-ignored untracked files, exclude provider evidence, and reject symbolic
links, Git links, path escapes, runner-internal directories, ignored files, and secret-like names.
The server signs the artifact role, filename, and exact deliverable metadata in addition to the
content digest. A run cannot pass verification until the ready source object links the exact
normalized verification digest to the exact exported-byte digest. The runner waits for durable
storage acknowledgment before worktree cleanup.

Post-verification commit creation is fixed runner behavior on the isolated task branch. It does not
authorize credential use, branch publication, pull-request creation, auto-merge, merge, or deploy.

Pull-request publication runs only in the trusted publisher CLI after a dedicated server
authorization check. GitHub credentials stay in the publisher's keyring or process environment;
they are never returned by the server, passed to the runner or producing agent, written into the
portable bundle, persisted in authorization/provenance records, or included in command arguments.
Publisher fencing tokens are opaque, expiring capabilities omitted from shared state and events.

Publication revalidates persisted verifier, deliverable, policy, budget, breaker, Corp, role, target,
base, branch, and source-issue authority before acquiring an attempt. The publisher accepts only an
exact signed Git bundle, never force-pushes a conflicting branch, refuses closed or auto-merge
pull requests, and records the pull-request identity before changing Project status. Duplicate,
restart, timeout, and external-success/local-failure recovery adopt only matching remote effects.
The database constrains auto-merge, merge authorization, and deployment authorization to false.

Before every branch push, pull-request creation, and Project mutation, lease renewal transactionally
rechecks the current attempt actor against its persisted role snapshot and reruns current run,
mission, requester, Corp-budget, and hard-breaker authority. Pull-request adoption requires
`isCrossRepository = false`, the target repository owner, and the exact verified head SHA. Remote
`HEAD` is accepted only when its symbolic branch target exists and advertises the same object ID.

## Required production boundaries

- Every persistent object is scoped to a Corp.
- Authorization runs before reads, writes, and subscriptions.
- Human identity uses OIDC and passkeys.
- Agent and runner identities are independent and revocable.
- The server does not execute untrusted shell commands.
- Runners connect outbound and receive scoped assignments.
- Long-lived secrets never enter prompts, logs, command arguments, or agent-readable files.
- Irreversible effects require authorization and idempotency.
- GitHub Project status, pull-request publication, merge, and deployment remain separate effects;
  no factory claim implicitly authorizes a later effect.
- Artifacts are content-hashed.
- Agent control uses rotating fencing tokens; stale tokens are rejected.
- Lease tokens are returned only when the current controller explicitly claims or renews control;
  they are omitted from shared snapshots, transfers, and event payloads.
- Emergency stop is role-gated and audited.
- Runner connections use epochs, and run assignments use independent private fencing tokens.
- Assignment tokens are omitted from shared snapshots and event payloads.
- Every runner event must match both the current connection epoch and the stored assignment token.
- A stale runner cannot turn a `lost` run back into an active or cancelled run.

## Reporting

Until private vulnerability reporting is enabled on the GitHub repository, report security issues
directly to the repository owner rather than opening a public issue with exploit details.
