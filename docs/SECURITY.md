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

These gaps are tracked in ECorp Build GitHub Project #3 and linked issues, not in
`docs/BACKLOG.md`, which is historical seed material only. They are not production claims.

Claude Code is launched in safe mode without user plugins, hooks, MCP servers, browser integration,
slash commands, or auto-memory. OpenCode uses its plugin-free `--pure` mode. These controls prevent
the customization leakage observed during the September 1, 2026 enterprise dogfood pass, but they
are only a partial boundary. Claude's supported stdio permission control protocol, manual
permission mode, and required initialize handshake now translate
tool requests to durable ECorp approvals without permission bypass, terminal scraping, or custom
stdin messages. GitHub issue #51 still tracks isolated provider homes, inherited-environment
allowlisting, stronger container isolation, and remaining real-provider recovery drills.

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
The controller's selected-Project-item lookup requires Corp `Operate` authorization and repeats the
store-side human operator-role check before returning source or policy metadata. Requests are
bounded to 1,000 Project item IDs of at most 160 characters and are deduplicated before the query.
The normalized Project owner, positive Project number, source kind, Corp, and item IDs all scope
the lookup, preventing an equal item ID in another Project from substituting its state or policy.
Guests and spectators receive no lookup results, and claim tokens remain excluded from lookup
responses. Claim and reclaim idempotency keys include the validated lease duration so a recovery
request with a changed lease cannot collide with a persisted request containing another duration.

Controllers renew their fenced lease immediately before a GitHub mutation and again before launch.
After each renewal they re-fetch and compare the Project item, issue revision, state, required
label, and dependency eligibility. A changed or newly blocked source is durably blocked before the
next effect. A final renewal follows each revalidation, and GitHub CLI subprocesses are killed on a
bounded timeout below the effect lease. The controller resolves the source ref in a checkout whose
GitHub remote matches the claimed repository, then persists the full 40- or 64-hex commit in policy.
Factory tasks and run launch records carry repository, ref, and commit as one all-or-nothing source
identity. The scheduler accepts only a runner advertising the same structured tuple, and the runner
independently rejects both start and resume commands before worktree access if any element differs.
Symbolic refs alone are never sufficient authority for factory write-capable routing. The browser
also requires an operator to select and confirm one connected runner's structured repository, ref,
and immutable commit before creating an ordinary mission. The server canonicalizes the selected
tuple from live runner capabilities, applies it to every planned task, and rejects adapter, model,
reasoning, repository, ref, or commit mismatch before mission persistence. Local repositories use
a stable opaque `local/<name>-<digest>` identity rather than an absolute host path. Legacy API
clients may omit source selection, but resume is always fenced to the source run's persisted
workspace base commit.

Pre-commit factory records are migrated only from unambiguous persisted workspace evidence.
Underivable legacy claims are not guessed or silently widened: migration marks them as requiring an
upgrade, and only the active fenced operator may resolve and persist a new commit before any run
exists. That dedicated idempotent operation is stored in `factory_operations` and emits
`factory.source_commit_pinned`; it rejects stale tokens, stale versions, already-pinned policies,
incompatible task contracts, and any mission that has already produced a run. New claim intake
requires a valid immutable commit and cannot self-declare the migration-only upgrade marker.
Recovery of a migration-marked record requires the controller's requested symbolic ref to exactly
match the persisted policy ref before resolving or storing a commit.

Factory-wide pause, resume, and reconciliation controls require an owner, admin, or manager and an
expected controller version plus idempotency key. Members may inspect controller health but cannot
change Corp-wide intake. Controller heartbeats are fenced by a rotating connection epoch, bounded
lease, service actor, and Corp. A stale process cannot extend the active lease or complete another
process's reconciliation generation. Controller status never exposes GitHub credentials or
work-item claim tokens.

Inline cockpit decisions do not weaken the underlying authorization boundary. Action approvals and
verification reviews retain their existing role, expiry, requester-exclusion, and idempotency
rules. Contextual comments remain ordinary durable room messages linked to the mission; comment
text cannot approve an effect, create a task, or steer a provider session implicitly. Comment
operation UUIDs are scoped to the Corp and exact normalized request; replay returns the existing
message, while changed content under the same key is rejected.

Cockpit steering uses the existing single-holder agent lease and lease token. The token is checked
before persistence but is never stored in the durable runner command. A separate client operation
UUID prevents retry from creating another control message, and the runner command ID fences
duplicate delivery across server reconnect. A monotonic lease version is persisted with the
command and rechecked before dispatch, so renewal, release, transfer, or expiry cancels a pending
stale steer. Negative runner acknowledgments also terminalize the command rather than leaving it
pending indefinitely. A browser that lost its private token must reclaim the same actor's lease,
which rotates the token and lease version before another steer. This does not add a second input
path or convert a comment into provider control.
External CLI failure details are collapsed to bounded single-line text before persistence so
multi-line stderr cannot bypass the durable blocked transition.

The GitHub Copilot permission handler automatically approves writes inside the assigned worktree,
read-only operations it can prove are scoped to that worktree, and reads from the SDK state
directory isolated to that worktree. It canonicalizes existing ancestors to reject symlink escapes.
External paths, network URLs, sandbox bypass, managed-policy approvals, and ambiguous shell
commands suspend durably. Shell approval cards include the bounded command text rather than only a
generic action label.

Claude permission requests use the same fail-closed worktree containment helper. Only recognized
read/write tools with an explicit contained path can be allowed locally. Existing ancestors are
canonicalized, so symlink escapes, traversal, unavailable worktree roots, blocked paths, Bash,
network tools, unknown tools, and malformed or oversized control frames cannot be auto-approved.
Suspended approval context preserves the request ID, tool-use ID, tool name, blocked path, decision
reason, title, display name, and description within durable event bounds. Raw structured input,
including Write/Edit content and secret-shaped values, remains only in process memory; durable
approval text records bounded field metadata and SHA-256 hashes. Approval returns the exact original
input only to the correlated provider request. Rejection and expiry return a bounded denial;
duplicate, unknown, cancelled, or mismatched requests fail closed, and a failed control-response
write fails the provider run.

External CLI provider roots are owned as complete process scopes rather than supervised as single
PIDs. Windows uses a private kill-on-close Job Object assigned while the provider is still
suspended. If assignment or resume setup fails, the suspended child remains under cleanup
supervision until its death is verified. Every terminal path invokes the same bounded
terminate-and-verify attempt and retries without dropping ownership when verification is uncertain.
The adapter does not return until the provider root is reaped and the owned scope reports no active
descendants. This prevents provider tools, plugin processes, or grandchildren from surviving an
interrupt while ECorp reports `provider_process_alive=false`. Unix external CLI adapters are
disabled because `setsid` or `setpgid` alone cannot prevent descendants from escaping the owned
scope. Timed-out availability probes retain one shared cleanup guardian per adapter and reject
duplicate probes until that guardian finishes, preventing unbounded detached cleanup tasks. This is
host-process containment, not a network or filesystem sandbox.

Mission descriptions, task contracts, and verifier policies are authority-bearing records.
Creation validates their bounds before persistence, and every revision stores both prior and
replacement values rather than rewriting history invisibly. Revision idempotency is scoped by Corp
and exact normalized request. New operations and replay require current mission-room membership;
the mission and task are locked while membership, actor role, active runs, expected version, and
mission budget are rechecked.

Pre-dispatch `redispatch` revisions are rejected after any run exists. `resume` revisions require
the latest terminal preserved provider/worktree checkpoint and reject any lineage that reached a
hard stop. Resume cannot change source identity, secret references, model, reasoning, budget, or
deliverable authority; it cannot widen tools or write paths or remove a prohibition. The revision
does not itself dispatch, preventing a contract mutation from implicitly authorizing execution.

Budget policies constrain run, mission, requester, and Corp usage. Repeated tools and explicit
no-progress events feed an auditable circuit breaker; ordinary human conversation does not.

Mission budget recovery is a dedicated owner/admin operation, not a resume parameter. Original
limits and consumed usage are never reset. Proposal and decision requests are Corp-scoped,
role-gated, versioned, and exactly idempotent; approval rechecks the current mission limits and
usage under lock before changing authority. Current mission-room membership is required for new
operations and idempotent replay, with the membership row protected against concurrent removal.

Finish-scope recovery cannot widen the task budget, write boundary, or verifier policy. Resume
scope paths reject absolute paths, traversal components, drive prefixes, backslashes, and
unsupported wildcard shapes before containment. The finish task must be the latest suspended task,
and approval fails if its contract or verifier policy changed after proposal.

Resume rejects exhausted mission, requester rolling, or Corp rolling authority before provider
dispatch and clamps the new run to all remaining limits. It accepts only the latest run in a
serialized provider-workspace lineage; any `stop` in that lineage permanently fences its
ancestors. Shared snapshots expose revision rationale and decisions for audit but no runner
assignment or fencing token. Browser-generated proposal and decision keys remain stable across a
lost response so retry replays one committed operation.

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
Commit/branch bundles use a unique temporary ref created only after the workspace branch is verified
against the generated head, then delete it after bundle creation. A detached worktree `HEAD` cannot
redirect or empty the bundle.

Pull-request publication runs only in the trusted publisher CLI after a dedicated server
authorization check. GitHub credentials stay in the publisher's keyring or process environment;
they are never returned by the server, passed to the runner or producing agent, written into the
portable bundle, persisted in authorization/provenance records, or included in command arguments.
Publisher fencing tokens are opaque, expiring capabilities omitted from shared state and events.
They are insufficient by themselves: each publication mutation also requires a separate enrolled
publisher workload credential. Enrollment and revocation require owner/admin `Manage` authority;
only credential hashes are stored. Corp, publisher ID, hash, expiry, and revocation are rechecked and
row-locked inside the same transaction that starts, renews, fails, or checkpoints publication. The
authenticated credential determines the publisher ID; a caller cannot choose another workload
identity while presenting a valid credential.

Publication revalidates persisted verifier, deliverable, policy, budget, breaker, Corp, role, target,
base, branch, and source-issue authority before acquiring an attempt. The publisher accepts only an
exact signed Git bundle, never force-pushes a conflicting branch, refuses closed or auto-merge
pull requests, and records the pull-request identity before changing Project status. Duplicate,
restart, timeout, and external-success/local-failure recovery adopt only matching remote effects.
The database constrains auto-merge, merge authorization, and deployment authorization to false.
The publisher reads its requested work item, publication, and mission deliverables through an exact
Corp-authorized context endpoint rather than trusting bounded shared snapshots. Exact publication
status reads apply the same mission-room join; knowing a work-item UUID does not expose pull-request
body, authorization reason/snapshot, publisher identity, or failure detail to another-room members.
The context's initial work-item lookup applies that join too, so source issue metadata, factory
policy, claim ownership, and work-item failure details are not disclosed.

Before every branch push, pull-request creation, and Project mutation, lease renewal transactionally
rechecks the current attempt actor against its persisted role snapshot and reruns current run,
mission, requester, Corp-budget, and hard-breaker authority while also reauthenticating the enrolled
publisher workload credential. Pull-request adoption requires `isCrossRepository = false`, the
target repository owner, and the exact verified head SHA. Remote `HEAD` is accepted only when its
symbolic branch target exists and advertises the same object ID. GitHub PR commands receive that
verified branch name rather than the literal `HEAD`, and the resolved PR base is retained separately
from the authorized symbolic base.
The publisher's mission-room membership is rechecked in the same transaction on new start,
idempotent replay, expired-lease recovery, and every renewal. Removing a still-manager actor from
the room therefore blocks branch, pull-request, and Project effects.

PR adoption also requires the exact authorized title and body. A collaborator-created PR with the
right branch and SHA but altered content is ignored and cannot advance Project state. Publication
retries remain pinned to the persisted deliverable ID even when the mission contains other
merge-ready outputs.
Pull-request URL validation requires canonical GitHub scheme, host, path shape, and number, while
owner/repository path components compare case-insensitively to GitHub's canonical casing.

Publication branches pass `git check-ref-format --branch` before the durable start request; the
server independently rejects invalid path components such as doubled separators and `.lock`
suffixes. Default start keys hash the complete normalized request, including publisher identity,
authorization reason, and lease duration, so changed recovery authority cannot reuse a request key
whose persisted operation has different fields. CLI-provided titles and bodies are normalized with
the same bounds and control-character rules as the server before plan comparison and idempotency.
Repository owner/name inputs are trimmed, validated, and lowercased at the same boundary.

GitHub Project status is read from the exact stored Project item node ID. The publisher verifies the
returned Project ID, owner, number, item ID, Status field ID, and field type before any mutation, so
truncated item listings cannot turn a present item into an apparent disappearance.
The Status field definition and option IDs come from an exact GraphQL lookup on the known Project
node rather than the first page of `field-list`.
Publication base policy is branch-only: the controller and store accept `HEAD`, short branch names,
or `refs/heads/*`, while rejecting tags, remote-tracking refs, invalid branch shapes, and control
characters before durable claim. When omitted, it derives from the selected source base ref rather
than independently assuming `HEAD` or `main`.

The resolved PR base can never also be the publication head branch. A read-only remote preflight
rejects that configuration before the server persists a publication, and the guard repeats before
push, so publication cannot mutate the default branch as a substitute for merge authorization.
Generated authorization IDs remain stable across restart and duplicate invocation. A different
recovery actor receives a new actor-bound ID rather than inheriting the first actor's grant. Local
body-file normalization matches server canonicalization so idempotency cannot fail on CRLF or a
trailing newline.
The CLI reads the publisher credential from a file before its first publication API mutation and
never places it in arguments, plans, output, events, snapshots, or durable publication records.

A retry of a durable `published` result performs no remote preflight or external effect. It returns
the persisted PR identity even if the PR was later merged or the base branch advanced.
For an unfinished publication, the publisher renews authority before remote reads, refreshes the
exact Project item, and re-fetches the checkpointed PR. It then renews authority a second time
immediately before Project mutation. Another PR check and authority renewal precede completion,
preventing a stale publisher or a closed, retargeted, edited, or auto-merge-enabled PR from
advancing or being recorded as reviewed.

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
