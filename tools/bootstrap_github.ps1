[CmdletBinding()]
param(
    [string]$Owner = 'shyamsridhar123',
    [string]$Repository = 'ecorp',
    [string]$ProjectTitle = 'ECorp Build'
)

$ErrorActionPreference = 'Stop'
$repo = "$Owner/$Repository"

$labels = @(
    @{ name = 'type:feature'; color = '1D76DB'; description = 'New product capability' },
    @{ name = 'type:bug'; color = 'D73A4A'; description = 'Behavior contradicting the product contract' },
    @{ name = 'type:research'; color = '5319E7'; description = 'Investigation or decision work' },
    @{ name = 'type:security'; color = 'B60205'; description = 'Security boundary or threat-model work' },
    @{ name = 'area:server'; color = '0E8A16'; description = 'Control-plane server' },
    @{ name = 'area:runner'; color = 'FBCA04'; description = 'Execution-plane runner' },
    @{ name = 'area:web'; color = '1D76DB'; description = 'Web and desktop experience' },
    @{ name = 'area:protocol'; color = '006B75'; description = 'Wire protocols and schemas' },
    @{ name = 'area:infra'; color = 'C2E0C6'; description = 'Build, deployment, and operations' },
    @{ name = 'area:docs'; color = '0075CA'; description = 'Product and engineering documentation' },
    @{ name = 'priority:p0'; color = 'B60205'; description = 'Required on the critical path' },
    @{ name = 'priority:p1'; color = 'D93F0B'; description = 'Important after the critical path' },
    @{ name = 'priority:p2'; color = 'FBCA04'; description = 'Useful follow-up' },
    @{ name = 'status:blocked'; color = '000000'; description = 'Cannot progress without an external dependency' },
    @{ name = 'evals'; color = '7057FF'; description = 'Evaluation and real-world verification' },
    @{ name = 'multiplayer'; color = '5319E7'; description = 'Multi-human realtime behavior' },
    @{ name = 'agent-runtime'; color = '0E8A16'; description = 'Agent adapters, execution, and isolation' }
)

foreach ($label in $labels) {
    gh label create $label.name `
        --repo $repo `
        --color $label.color `
        --description $label.description `
        --force | Out-Null
}

$milestones = @(
    @{ title = 'M0 — Foundation'; description = 'Product contract, architecture, durable event substrate, local stack, and deterministic vertical slice.'; due = '2026-09-02T23:59:59Z' },
    @{ title = 'M1 — Multiplayer vertical slice'; description = 'Resumable realtime, rooms, hardened control leases, runner reconciliation, and desktop shell.'; due = '2026-09-20T23:59:59Z' },
    @{ title = 'M2 — Real agent work'; description = 'Real provider adapters, worktrees, task graphs, and evidence-gated completion.'; due = '2026-10-11T23:59:59Z' },
    @{ title = 'M3 — Safety and trust'; description = 'Authentication, runner enrollment, secrets, approvals, budgets, and artifact provenance.'; due = '2026-11-01T23:59:59Z' },
    @{ title = 'M4 — Alpha'; description = 'Cross-platform validation, protocol conformance, chaos tests, naming review, and dogfood.'; due = '2026-11-29T23:59:59Z' }
)

$existingMilestones = @(
    gh api "repos/$repo/milestones?state=all&per_page=100" | ConvertFrom-Json
)
foreach ($milestone in $milestones) {
    if ($existingMilestones.title -contains $milestone.title) {
        continue
    }
    @{
        title = $milestone.title
        description = $milestone.description
        due_on = $milestone.due
    } |
        ConvertTo-Json -Compress |
        gh api --method POST "repos/$repo/milestones" --input - |
        Out-Null
}

$projects = (gh project list --owner $Owner --format json --limit 100 | ConvertFrom-Json).projects
$project = $projects | Where-Object title -eq $ProjectTitle | Select-Object -First 1
if (-not $project) {
    $project = gh project create --owner $Owner --title $ProjectTitle --format json |
        ConvertFrom-Json
}
gh project edit $project.number `
    --owner $Owner `
    --description 'Execution board for the ECorp product, architecture, security, implementation, and real-world validation plan.' `
    --readme 'Backlog source: `docs/BACKLOG.md`. Product contract: `docs/PRODUCT_AND_TECHNICAL_PLAN.md`.' |
    Out-Null

$issues = @(
    @{
        milestone = 'M0 — Foundation'
        title = 'Establish the product contract and architecture decision records'
        labels = @('type:feature', 'area:docs', 'priority:p0')
        outcome = 'The repository has one authoritative product plan and explicit architecture, security, evaluation, and licensing decisions.'
        acceptance = @(
            'Product and technical plan is committed.',
            'Architecture, security, threat model, and eval documents are committed.',
            'ADRs 0001 through 0008 are accepted and linked from the README.'
        )
    },
    @{
        milestone = 'M0 — Foundation'
        title = 'Create durable Postgres state and an idempotent event journal'
        labels = @('type:feature', 'area:server', 'priority:p0')
        outcome = 'Corp, actor, room, agent, mission, task, run, lease, message, and event state survive process and browser restarts.'
        acceptance = @(
            'Database migrations create all first-slice aggregates.',
            'Meaningful mutations append an event in the same transaction.',
            'Duplicate runner event IDs do not duplicate state transitions.',
            'A bounded Corp snapshot can be queried by the web client.'
        )
    },
    @{
        milestone = 'M0 — Foundation'
        title = 'Build the deterministic child-process agent adapter'
        labels = @('type:feature', 'area:runner', 'agent-runtime', 'priority:p0')
        outcome = 'The execution plane proves a real server-to-runner-to-child-process-to-artifact path without provider nondeterminism.'
        acceptance = @(
            'Runner launches a real process in a run-specific workspace.',
            'Structured status and output events reach the server.',
            'The child writes result.md.',
            'Runner computes and reports a matching SHA-256 digest.'
        )
    },
    @{
        milestone = 'M0 — Foundation'
        title = 'Provide one-command local start, stop, and end-to-end smoke testing'
        labels = @('type:feature', 'area:infra', 'evals', 'priority:p0')
        outcome = 'A contributor can launch and verify the complete local stack without manually coordinating processes.'
        acceptance = @(
            'start_local.ps1 verifies ports, Postgres, server, runner, and web health.',
            'stop_local.ps1 terminates only processes recorded for this workspace.',
            'e2e_smoke.ps1 proves lease conflict, live control, execution, artifact, hash, and terminal state.',
            'CI uploads runtime evidence.'
        )
    },
    @{
        milestone = 'M1 — Multiplayer vertical slice'
        title = 'Implement resumable browser event streams with sequence cursors'
        labels = @('type:feature', 'area:protocol', 'area:web', 'multiplayer', 'priority:p0')
        outcome = 'A reconnecting browser resumes from its last event sequence without gaps or duplicate application.'
        acceptance = @(
            'WebSocket handshake accepts an after-sequence cursor.',
            'Server replays committed events before switching to live delivery.',
            'Lagged subscribers recover through replay.',
            'Reconnect and duplicate tests pass.'
        )
    },
    @{
        milestone = 'M1 — Multiplayer vertical slice'
        title = 'Harden control leases with release, transfer, fencing, and emergency stop'
        labels = @('type:feature', 'area:server', 'area:web', 'multiplayer', 'priority:p0')
        outcome = 'Human and automated controllers cannot issue contradictory live commands to one agent session.'
        acceptance = @(
            'Lease release and explicit transfer are supported.',
            'Every live control command carries a fencing token.',
            'A stale controller is rejected.',
            'Authorized emergency stop bypasses ordinary ownership.',
            'Concurrent claim tests pass.'
        )
    },
    @{
        milestone = 'M1 — Multiplayer vertical slice'
        title = 'Add durable rooms and threaded human-agent messages'
        labels = @('type:feature', 'area:server', 'area:web', 'multiplayer', 'priority:p0')
        outcome = 'People and agents converse in the same persistent room and can link messages to missions, tasks, runs, and artifacts.'
        acceptance = @(
            'Messages and replies persist with author attribution.',
            'Both browser sessions receive updates in real time.',
            'Mentions and task/run links are structured.',
            'Room membership gates reads and writes.'
        )
    },
    @{
        milestone = 'M1 — Multiplayer vertical slice'
        title = 'Add runner heartbeat, disconnect grace, and run reconciliation'
        labels = @('type:feature', 'area:runner', 'area:server', 'priority:p0')
        outcome = 'A transient runner or network failure does not silently lose, duplicate, or prematurely fail active work.'
        acceptance = @(
            'Runner heartbeat timestamps are persisted.',
            'Disconnect enters a bounded grace state.',
            'Reconnect presents active run IDs and fencing tokens.',
            'Server resolves completed, resumed, and lost runs deterministically.'
        )
    },
    @{
        milestone = 'M1 — Multiplayer vertical slice'
        title = 'Package a thin Tauri desktop client'
        labels = @('type:feature', 'area:web', 'priority:p1')
        outcome = 'Desktop users get a native shell without moving process supervision or authoritative state into the client.'
        acceptance = @(
            'Desktop uses the same web application and APIs.',
            'Closing the desktop leaves active runs alive.',
            'Deep links open the correct Corp, room, task, or run.',
            'Windows packaging smoke test passes.'
        )
    },
    @{
        milestone = 'M2 — Real agent work'
        title = 'Introduce the AgentAdapter lifecycle contract'
        labels = @('type:feature', 'area:runner', 'area:protocol', 'agent-runtime', 'priority:p0')
        outcome = 'Provider-specific runtimes implement one tested spawn, stream, steer, interrupt, stop, resume, usage, and artifact contract.'
        acceptance = @(
            'Fake adapter moves behind the trait.',
            'Capabilities are reported by adapter.',
            'Lifecycle conformance tests are provider-independent.',
            'Unsupported features degrade explicitly.'
        )
    },
    @{
        milestone = 'M2 — Real agent work'
        title = 'Implement the first real OpenAI Codex adapter'
        labels = @('type:feature', 'area:runner', 'agent-runtime', 'priority:p0')
        outcome = 'A real Codex session can execute a bounded repository task under ECorp supervision.'
        acceptance = @(
            'Start and resume are supported.',
            'Terminal and structured lifecycle events stream to ECorp.',
            'Steer, interrupt, and stop have verified behavior.',
            'Usage and completion evidence are recorded.',
            'A real sample repository change passes end-to-end.'
        )
    },
    @{
        milestone = 'M2 — Real agent work'
        title = 'Add per-task git worktree isolation and safe cleanup'
        labels = @('type:feature', 'area:runner', 'agent-runtime', 'priority:p0')
        outcome = 'Parallel write-capable agents never share or mutate the configured checkout.'
        acceptance = @(
            'Each run receives a dedicated worktree and branch.',
            'Resolved paths remain inside the configured workspace root.',
            'Unintegrated work is preserved.',
            'Cleanup requires verified safe state.',
            'Windows path-with-spaces tests pass.'
        )
    },
    @{
        milestone = 'M2 — Real agent work'
        title = 'Implement bounded task-graph planning and scheduling'
        labels = @('type:feature', 'area:server', 'agent-runtime', 'priority:p0')
        outcome = 'A manager strategy turns a mission into explicit dependency-aware task contracts and assignments.'
        acceptance = @(
            'Task graph has depth, node, retry, and budget limits.',
            'Every task has objective, output, tools, boundaries, and acceptance tests.',
            'Capability and availability matching is deterministic.',
            'The manager is a replaceable strategy rather than a singleton.'
        )
    },
    @{
        milestone = 'M2 — Real agent work'
        title = 'Add evidence-gated verifier policies'
        labels = @('type:feature', 'area:server', 'evals', 'priority:p0')
        outcome = 'An agent cannot complete a task by merely claiming it is done.'
        acceptance = @(
            'Verifier policies support files, commands, tests, schemas, screenshots, and human approval.',
            'Evidence is linked to the task and run.',
            'Failed verification returns the task to an explicit state.',
            'Independent reviewer policy is supported.'
        )
    },
    @{
        milestone = 'M2 — Real agent work'
        title = 'Add Claude Code and OpenCode adapters'
        labels = @('type:feature', 'area:runner', 'agent-runtime', 'priority:p1')
        outcome = 'ECorp proves the adapter architecture across multiple independent coding-agent runtimes.'
        acceptance = @(
            'Both adapters pass lifecycle conformance tests.',
            'Provider-specific permission and resume behavior is documented.',
            'A common sample task produces equivalent evidence.'
        )
    },
    @{
        milestone = 'M3 — Safety and trust'
        title = 'Add OIDC or passkey authentication and Corp-scoped RBAC'
        labels = @('type:security', 'area:server', 'area:web', 'priority:p0')
        outcome = 'Production mode has recoverable human identity and authorization before every read, write, and subscription.'
        acceptance = @(
            'Fixed demo identities are development-only.',
            'Owner, admin, manager, member, guest, and spectator roles are enforced.',
            'Cross-Corp access tests fail closed.',
            'WebSocket subscriptions are authorized before replay.'
        )
    },
    @{
        milestone = 'M3 — Safety and trust'
        title = 'Add runner enrollment and rotating workload identity'
        labels = @('type:security', 'area:runner', 'area:server', 'priority:p0')
        outcome = 'Only enrolled, non-revoked runner nodes can receive work or publish run events.'
        acceptance = @(
            'Enrollment is explicit and auditable.',
            'Runner uses a short-lived certificate or equivalent credential.',
            'Revocation takes effect without reinstalling the server.',
            'Unknown and replayed identities are rejected.'
        )
    },
    @{
        milestone = 'M3 — Safety and trust'
        title = 'Build the scoped secret broker'
        labels = @('type:security', 'area:runner', 'agent-runtime', 'priority:p0')
        outcome = 'Agents use task-scoped capabilities without receiving long-lived workspace credentials.'
        acceptance = @(
            'Tasks reference secrets by identifier.',
            'Broker enforces actor, task, tool, resource, and expiry scope.',
            'Values are absent from prompts, arguments, logs, events, and memory.',
            'Reduced-assurance environment-only adapters are labeled.'
        )
    },
    @{
        milestone = 'M3 — Safety and trust'
        title = 'Implement durable approval suspension and exactly-once resume'
        labels = @('type:feature', 'type:security', 'area:server', 'multiplayer', 'priority:p0')
        outcome = 'Risky work suspends durably and resumes or cancels exactly once after an authorized decision.'
        acceptance = @(
            'Approval request records risk, requester, action, required role, expiry, and rationale.',
            'Server restart does not lose suspended work.',
            'Duplicate approvals do not duplicate effects.',
            'A second device can approve and resume the active run.'
        )
    },
    @{
        milestone = 'M3 — Safety and trust'
        title = 'Add budgets and the steer-constrain-suspend-stop circuit breaker'
        labels = @('type:feature', 'type:security', 'area:server', 'area:runner', 'priority:p0')
        outcome = 'Runaway spend, recursive delegation, repeated tools, and no-progress loops are bounded without interrupting healthy conversation.'
        acceptance = @(
            'Per-run, mission, actor, and Corp budgets are supported.',
            'Breaker inputs and escalation are auditable.',
            'Healthy human conversation is not treated as no progress.',
            'Loop and spend scenarios are covered by deterministic tests.'
        )
    },
    @{
        milestone = 'M3 — Safety and trust'
        title = 'Move artifacts to object storage with signed provenance'
        labels = @('type:security', 'area:server', 'area:infra', 'priority:p1')
        outcome = 'Shared state references durable content-addressed artifacts rather than runner-local file paths.'
        acceptance = @(
            'Runner uploads to S3-compatible storage.',
            'Server verifies digest and declared media type.',
            'Artifact records include producer, run, task, verifier, and retention.',
            'Download authorization is Corp-scoped.'
        )
    },
    @{
        milestone = 'M4 — Alpha'
        title = 'Build the 100-scenario agent and systems evaluation suite'
        labels = @('type:feature', 'evals', 'priority:p0')
        outcome = 'Architecture and agent behavior can be compared through repeatable outcome, safety, cost, and intervention measurements.'
        acceptance = @(
            'At least 100 versioned scenarios exist.',
            'Deterministic and real-provider lanes are separated.',
            'Reports include success, verified completion, rework, cost, latency, and intervention.',
            'Regression thresholds run in CI where deterministic.'
        )
    },
    @{
        milestone = 'M4 — Alpha'
        title = 'Add chaos, restart, duplicate-delivery, and reconnect testing'
        labels = @('type:feature', 'evals', 'area:infra', 'priority:p0')
        outcome = 'The system proves durability under the failure cases listed in docs/EVALS.md.'
        acceptance = @(
            'Server restart during a run is tested.',
            'Runner disconnect and recovery is tested.',
            'Duplicate events and launches are tested.',
            'Browser reconnect and sequence replay is tested.',
            'Evidence is retained as CI artifacts.'
        )
    },
    @{
        milestone = 'M4 — Alpha'
        title = 'Validate runners on Windows, macOS, and Linux'
        labels = @('type:feature', 'area:runner', 'evals', 'priority:p0')
        outcome = 'The supported runner contract has real evidence on all three operating systems.'
        acceptance = @(
            'Process-tree cleanup is verified on each OS.',
            'Paths with spaces and Unicode are covered.',
            'PTY, worktree, interrupt, and artifact behavior are exercised.',
            'Platform-specific limitations are documented.'
        )
    },
    @{
        milestone = 'M4 — Alpha'
        title = 'Add MCP, ACP, and A2A gateways with conformance tests'
        labels = @('type:feature', 'area:protocol', 'agent-runtime', 'priority:p1')
        outcome = 'ECorp interoperates through open protocols without leaking internal task and policy semantics into their schemas.'
        acceptance = @(
            'MCP exposes scoped ECorp tools and context.',
            'ACP adapter covers compatible local agent sessions.',
            'A2A gateway supports discovery, tasks, messages, and streaming.',
            'Protocol conformance tests and version negotiation are documented.'
        )
    },
    @{
        milestone = 'M4 — Alpha'
        title = 'Complete public name, trademark, domain, art, and licensing review'
        labels = @('type:research', 'area:docs', 'priority:p1')
        outcome = 'The public brand can launch without knowingly colliding with existing ECorp products or copying protected visual identity.'
        acceptance = @(
            'Trademark and app-store search is recorded.',
            'Domain, package, and social-handle options are recorded.',
            'Original art direction is approved.',
            'NOTICE and third-party provenance are audited.'
        )
    },
    @{
        milestone = 'M4 — Alpha'
        title = 'Dogfood the alpha with three real teams'
        labels = @('type:research', 'evals', 'multiplayer', 'priority:p1')
        outcome = 'Real teams complete real missions and produce measured evidence about setup, trust, coordination, and rework.'
        acceptance = @(
            'Three teams complete onboarding.',
            'Each runs at least one multi-human and multi-agent mission.',
            'Setup time, success, recovery, rework, and intervention are measured.',
            'Findings produce prioritized follow-up issues.'
        )
    }
)

$existingIssues = @(
    gh issue list --repo $repo --state all --limit 500 --json title,url |
        ConvertFrom-Json
)
$created = @()
$skipped = @()

foreach ($issue in $issues) {
    $existing = $existingIssues | Where-Object title -eq $issue.title | Select-Object -First 1
    if ($existing) {
        $url = $existing.url
        $skipped += $issue.title
    } else {
        $criteria = ($issue.acceptance | ForEach-Object { "- [ ] $_" }) -join "`n"
        $body = @(
            '## Outcome'
            ''
            $issue.outcome
            ''
            '## Acceptance criteria'
            ''
            $criteria
            ''
            '## Source'
            ''
            '- `docs/BACKLOG.md`'
            '- `docs/PRODUCT_AND_TECHNICAL_PLAN.md`'
        ) -join "`n"
        $url = gh issue create `
            --repo $repo `
            --title $issue.title `
            --body $body `
            --label ($issue.labels -join ',') `
            --milestone $issue.milestone
        $url = ($url | Select-Object -Last 1).Trim()
        $created += $url
    }
    gh project item-add $project.number --owner $Owner --url $url | Out-Null
}

[ordered]@{
    repository = "https://github.com/$repo"
    project = $project.url
    project_number = $project.number
    created_issue_count = $created.Count
    skipped_issue_count = $skipped.Count
    created_issues = $created
} | ConvertTo-Json -Depth 10
