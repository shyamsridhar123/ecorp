// Offline only: receipts, history, child execution, and suite I/O are in memory.
// Never open a runtime receipt, create filesystem fixtures, read process.env,
// execute the native CLI/provider, or contact any endpoint from these tests.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { runInNewContext } from 'node:vm'
import test from 'node:test'
import * as driver from './e2e_checkpoint_verification.mjs'

const id = (n) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`
const clone = (value) => structuredClone(value)
const sha = (value) => createHash('sha256').update(value).digest('hex')
const canonical = (value) => JSON.stringify(value, (_, item) =>
  item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.keys(item).sort().map((key) => [key, item[key]])) : item)
const hashJson = (value) => sha(canonical(value))
const WIN = path.win32
const SENTINEL = 'SYNTHETIC_PRIVATE_TOKEN_AND_PROVIDER_OUTPUT_DO_NOT_PERSIST'
const FIXTURE_SHA = 'c'.repeat(64) // Explicit current attestation; no fixture-file read.
const BASE_SHA = 'f34848ca92665c342abd5816c9e3eda0e82180671195362bcd0080544a3bc2ac'
const EXPECTED = {
  server: 'http://127.0.0.1:18574',
  repository: 'shyamsridhar123/ecorp-enterprise-lab',
  source: 'C:\\Users\\shyamsridhar\\.codex\\dogfood\\issue174-startup-20260907\\source',
  runner_root: 'C:\\Users\\shyamsridhar\\.codex\\dogfood\\issue174-startup-20260907\\runner-workspaces',
  runtime: 'C:\\Users\\shyamsridhar\\.codex\\dogfood\\issue195-checkpoint-runtime-20260908',
  workspace: 'C:\\Users\\shyamsridhar\\.codex\\worktrees\\ecorp-issue174-local-start',
  source_commit: 'a8894b5f02d56f10e2da38df47a450ff71e92fbe',
  code_commit: 'e291a6e19c0b5079ce7260a2fe05c4eb451165c4',
  runner: 'issue174-local-start',
  server_sha256: 'b39e4306d1526a674c57c08091a3b3d9599e4c10609c2fd8b7df0d107c0b80c0',
  runner_sha256: 'b20d6ed77ef7355a2f8f62aa3496186bc38f539b50214e841bbc95698525fd56',
  cli_sha256: '045aa20771f603f91026eb4755c4df8198404a6aaeaf274d3ba6ea280bfb8a1c',
  git: 'C:\\Program Files\\Git\\cmd\\git.exe',
}
const EXTRA = ['factory_work_items', 'factory_verification_recoveries', 'verification_evidence',
  'verification_requests', 'source_deliverables', 'mission_contract_revisions',
  'mission_budget_revisions', 'action_approvals', 'circuit_breaker_incidents']

function exported(name) {
  assert.equal(typeof driver[name], 'function', `parent driver export ${name} is not implemented yet`)
  return driver[name]
}

function receipt() {
  const binaries = Object.fromEntries(['server', 'runner'].map((role) => [role, {
    path: WIN.join(EXPECTED.runtime, 'bin', `crony-${role}.exe`),
    sha256: EXPECTED[`${role}_sha256`],
  }]))
  return {
    schema_version: 1, test_owned: true, issue: 195, owner_task: id(900),
    phase: 'running', server_url: EXPECTED.server, corp_id: id(1), actor_id: id(17),
    runner_id: EXPECTED.runner, runner_root: EXPECTED.runner_root,
    workspace: EXPECTED.workspace, source_commit: EXPECTED.code_commit,
    source_repository_path: EXPECTED.source, source_base_ref: 'HEAD',
    source_base_commit: EXPECTED.source_commit, binaries,
    processes: Object.fromEntries(['server', 'runner'].map((role, index) => [role, {
      role, pid: 50_000 + index, executable: binaries[role].path,
      workspace: EXPECTED.workspace, started_utc: '2026-09-08T16:00:00.000Z',
      stdout: SENTINEL, stderr: SENTINEL,
    }])),
    observation_failures: [{ cause: SENTINEL }],
    unused_metadata: { claim_token: SENTINEL, assignment_token: SENTINEL },
  }
}

function options(overrides = {}) {
  return {
    receiptPath: WIN.join(EXPECTED.runtime, 'runtime.json'),
    outputDir: WIN.join(EXPECTED.runtime, 'offline-owned-output'),
    cli: WIN.join(EXPECTED.runtime, 'bin', 'crony-cli.exe'),
    fixtureSha256: FIXTURE_SHA, reviewer: id(18), ...overrides,
  }
}

const config = (overrides = {}, metadata = receipt()) =>
  driver.configuration(metadata, options(overrides), WIN)
const requiredPairs = () => [
  ['--receipt', options().receiptPath], ['--output-dir', options().outputDir],
  ['--cli', options().cli], ['--fixture-sha256', FIXTURE_SHA],
  ['--reviewer-actor-id', id(18)],
]

function emptyState(c = config()) {
  return {
    snapshot: {
      ...Object.fromEntries(['missions', 'tasks', 'runs', 'agents', 'events', ...EXTRA]
        .map((name) => [name, []])),
      corp: { id: c.corp_id },
      actors: [
        { id: c.actor_id, corp_id: c.corp_id, kind: 'human', role: 'owner' },
        { id: c.reviewer, corp_id: c.corp_id, kind: 'human', role: 'member', display_name: 'Bob' },
      ],
      rooms: [{ id: id(20), corp_id: c.corp_id }],
    },
    runners: [{
      id: c.runner_id, corp_id: c.corp_id, connected: true, status: 'connected',
      capabilities: [
        { name: 'codex', available: true },
        { name: 'workspace-isolation', available: true, source_repository: c.source.repository,
          source_base_ref: c.source.base_ref, source_base_commit: c.source.base_commit },
      ],
    }],
  }
}

function event(c, seq, overrides = {}) {
  return {
    id: id(10_000 + seq), seq, schema_version: 1, corp_id: c.corp_id,
    room_id: id(20), actor_id: null, type: 'run.output', aggregate_type: 'run',
    aggregate_id: id(32), aggregate_version: 1, correlation_id: id(30), causation_id: null,
    idempotency_key: `offline-event-${seq}`, visibility: 'room',
    payload: { text: SENTINEL, assignment_token: SENTINEL, claim_token: SENTINEL },
    created_at: '2026-09-08T16:00:00.000Z', ...overrides,
  }
}

function history(c = config(), count = 301) {
  const state = emptyState(c)
  state.snapshot.missions.push({ id: id(30), corp_id: c.corp_id, status: 'failed', description: SENTINEL })
  state.snapshot.tasks.push({ id: id(31), corp_id: c.corp_id, mission_id: id(30), status: 'failed' })
  state.snapshot.runs.push({ id: id(32), corp_id: c.corp_id, task_id: id(31), status: 'failed', summary: SENTINEL })
  for (const [index, table] of EXTRA.entries()) {
    state.snapshot[table].push({
      ...(table === 'verification_requests' ? { run_id: id(32) } : { id: id(40 + index) }),
      corp_id: c.corp_id, mission_id: id(30), task_id: id(31), status: 'failed', detail: SENTINEL,
    })
  }
  const events = Array.from({ length: count }, (_, i) => event(c, i + 1))
  state.snapshot.events = events.slice(-2)
  return { state, replay: { events, through: count } }
}

function assertOwnedFile(file, outputDir) {
  const relative = WIN.relative(outputDir, file)
  assert.ok(relative && relative !== '..' && !relative.startsWith('..\\') && !WIN.isAbsolute(relative))
  assert.equal(WIN.extname(file), '.json')
}

test('issue195 public receipt binds parent binaries, pinned runtime identity, and distinct task source', () => {
  for (const [field, value] of Object.entries(EXPECTED)) {
    if (!['server_sha256', 'runner_sha256'].includes(field)) assert.equal(driver.PIN[field], value)
  }
  const metadata = receipt()
  const before = clone(metadata)
  const c = config({}, metadata)
  assert.deepEqual(metadata, before)
  assert.equal(c.receipt_path, WIN.join(EXPECTED.runtime, 'runtime.json'))
  assert.equal(c.receipt_id, metadata.owner_task)
  assert.equal(c.server_url, EXPECTED.server)
  assert.deepEqual(c.source, { repository: EXPECTED.repository, base_ref: 'HEAD', base_commit: EXPECTED.source_commit })
  assert.equal(c.cli_sha256, EXPECTED.cli_sha256)
  assert.equal(c.runtime_binaries.server.sha256, EXPECTED.server_sha256)
  assert.equal(c.runtime_binaries.runner.sha256, EXPECTED.runner_sha256)
  assert.equal(c.provider.sha256, FIXTURE_SHA)
  assert.equal(c.reviewer, id(18))
  assert.equal(c.tokens, 5000)
  assert.equal(c.missing_artifact, false)
  assert.equal(c.timeout_ms, 180_000)
  assert.equal(c.poll_ms, 250)
  assert.equal(c.settle_ms, 3000)
  assert.ok(!JSON.stringify(c).includes(SENTINEL))
})

test('explicit current fixture hash, suspend tokens, and missing-artifact switch normalize without disk reads', () => {
  const c = config({ tokens: 6000, missingArtifact: true, fixtureSha256: 'D'.repeat(64) })
  assert.equal(c.tokens, 6000)
  assert.equal(c.missing_artifact, true)
  assert.equal(c.provider.sha256, 'd'.repeat(64))
})

test('rebuilt server/runner receipt hashes and an explicit rebuilt CLI hash remain valid and bound', () => {
  const metadata = receipt()
  metadata.binaries.server.sha256 = 'A'.repeat(64)
  metadata.binaries.runner.sha256 = 'B'.repeat(64)
  const c = config({ cliSha256: 'E'.repeat(64) }, metadata)
  assert.equal(c.runtime_binaries.server.sha256, 'a'.repeat(64))
  assert.equal(c.runtime_binaries.runner.sha256, 'b'.repeat(64))
  assert.equal(c.cli_sha256, 'e'.repeat(64))
  assert.equal(c.runtime_binaries.server.path, metadata.binaries.server.path)
  assert.equal(c.runtime_binaries.runner.path, metadata.binaries.runner.path)
  assert.equal(config().cli_sha256, EXPECTED.cli_sha256)
  const report = driver.newReport(c, id(901))
  const changed = clone(metadata)
  changed.binaries.runner.sha256 = 'f'.repeat(64)
  assert.throws(() => driver.validateSavedReport(report, config({ cliSha256: 'e'.repeat(64) }, changed)),
    { code: 'saved_report_binding_mismatch' })
})

test('all required options are explicit; help is inert', () => {
  assert.deepEqual(driver.parseArgs(['--help']), { help: true })
  assert.deepEqual(driver.parseArgs(requiredPairs().flat()), options())
  for (let index = 0; index < requiredPairs().length; index += 1) {
    assert.throws(() => driver.parseArgs(requiredPairs().filter((_, i) => i !== index).flat()),
      { code: 'explicit_receipt_output_cli_fixture_reviewer_required' })
  }
  assert.throws(() => driver.parseArgs([]))
})

test('every duplicate value option and duplicate boolean switch fails closed', () => {
  for (const pair of [...requiredPairs(), ['--cli-sha256', 'e'.repeat(64)], ['--tokens', '5000'], ['--timeout-ms', '15000'],
    ['--poll-ms', '250'], ['--settle-ms', '2000']]) {
    const args = requiredPairs().some(([name]) => name === pair[0])
      ? [...requiredPairs().flat(), ...pair]
      : [...requiredPairs().flat(), ...pair, ...pair]
    assert.throws(() => driver.parseArgs(args), { code: 'unknown_or_duplicate_option' })
  }
  for (const flag of ['--continue', '--missing-required-artifact', '--review-via-browser']) {
    assert.throws(() => driver.parseArgs([...requiredPairs().flat(), flag, flag]),
      { code: 'unknown_or_duplicate_option' })
  }
})

test('unknown targets, positionals, missing operands, and equals-form options are rejected', () => {
  for (const extra of [['--server', 'http://127.0.0.1:8791'], ['--reset'], ['--bootstrap'], ['--review-via-api'],
    ['--tokens=5000'], ['unexpected'], ['--tokens'], ['--tokens', '--continue'], ['--tokens', '']]) {
    assert.throws(() => driver.parseArgs([...requiredPairs().flat(), ...extra]))
  }
  const parsed = driver.parseArgs([...requiredPairs().flat(), '--tokens', '6000', '--timeout-ms', '15000',
    '--poll-ms', '200', '--settle-ms', '2000', '--continue', '--missing-required-artifact',
    '--review-via-browser', '--cli-sha256', 'e'.repeat(64)])
  assert.equal(parsed.tokens, 6000)
  assert.equal(parsed.timeoutMs, 15000)
  assert.equal(parsed.continuation, true)
  assert.equal(parsed.missingArtifact, true)
  assert.equal(parsed.reviewViaBrowser, true)
  assert.equal(parsed.cliSha256, 'e'.repeat(64))
})

for (const [label, mutate] of [
  ['old issue190', (r) => { r.issue = 190 }],
  ['string issue number', (r) => { r.issue = '195' }],
  ['parent rather than candidate code', (r) => { r.source_commit = '8317851b1c5733dc30f41eb29d9ff1e1b4ffa7a8' }],
  ['runtime commit used as task source', (r) => { r.source_base_commit = EXPECTED.code_commit }],
  ['unpinned source ref', (r) => { r.source_base_ref = 'main' }],
  ['another runner', (r) => { r.runner_id = 'runner-local' }],
  ['unowned receipt', (r) => { r.test_owned = false }],
  ['not-running supervisor', (r) => { r.phase = 'starting' }],
  ['missing owner', (r) => { delete r.owner_task }],
  ['invalid owner', (r) => { r.owner_task = 'not-a-uuid' }],
  ['changed code checkout', (r) => { r.workspace += '-other' }],
  ['changed source checkout', (r) => { r.source_repository_path += '-other' }],
  ['changed worktree root', (r) => { r.runner_root += '-other' }],
  ['invalid server digest', (r) => { r.binaries.server.sha256 = 'not-a-sha256' }],
  ['invalid runner digest', (r) => { r.binaries.runner.sha256 = 'g'.repeat(64) }],
  ['executable substitution', (r) => { r.processes.runner.executable += '.other.exe' }],
  ['binary outside owned bin', (r) => {
    r.binaries.runner.path = WIN.join(EXPECTED.runtime, 'elsewhere', 'runner.exe')
    r.processes.runner.executable = r.binaries.runner.path
  }],
  ['missing public process', (r) => { delete r.processes.server }],
  ['wrong process role', (r) => { r.processes.server.role = 'runner' }],
  ['wrong process workspace', (r) => { r.processes.runner.workspace = EXPECTED.source }],
  ['invalid process pid', (r) => { r.processes.runner.pid = 0 }],
  ['invalid process start', (r) => { r.processes.runner.started_utc = 'not-a-date' }],
]) {
  test(`configuration rejects ${label}`, () => {
    const r = receipt()
    mutate(r)
    assert.throws(() => config({}, r))
  })
}

function recoveryFixture(c = config(), { approved = false, exportFailure = false } = {}) {
  const f = originalFixture(c)
  const original = exported('assessOriginal')(f.state, f.replay, c, f.plan, f.identity)
  const recoveryId = id(109)
  const replacementId = id(110)
  const failed = exportFailure || c.missing_artifact
  const replacement = {
    ...clone(f.run), id: replacementId, execution_mode: 'verification_only',
    resumed_from_run_id: f.run.id, provider_session_id: null, breaker_stage: 'healthy',
    budget_tokens_limit: 0, budget_cost_microusd_limit: 0, input_tokens: 0,
    status: failed ? 'failed' : approved ? 'completed' : 'waiting_for_approval',
    verification_status: failed ? 'failed' : approved ? 'passed' : 'awaiting_approval',
    verification_sha256: failed ? null : 'd'.repeat(64), deliverable_sha256: null,
  }
  const recovery = {
    id: recoveryId, corp_id: c.corp_id, factory_work_item_id: f.item.id,
    source_run_id: f.run.id, replacement_run_id: replacementId,
    mission_id: f.mission.id, task_id: f.task.id, mode: 'checkpoint_verification',
    authorized_by: c.actor_id, reason: f.plan.reason, contract_revision_id: null,
    observed_source_revision: f.plan.issue.updatedAt,
    previous_verification_policy: clone(f.plan.policy), replacement_verification_policy: clone(f.plan.policy),
    status: failed ? 'failed' : approved ? 'completed' : 'running',
  }
  const context = {
    checkpoint_verification: true, source_run_id: f.run.id, task_id: f.task.id,
    mission_id: f.mission.id, work_item_id: f.item.id, expected_head_commit: EXPECTED.source_commit,
    workspace_fingerprint: f.proof.workspace_fingerprint, remaining_mission_tokens: 0,
    remaining_attempts: 1, remaining_mission_cost_microusd: 1_000_000,
  }
  f.state.snapshot.runs.push(replacement)
  f.state.snapshot.factory_verification_recoveries.push(recovery)
  f.item.state = failed ? 'verification_failed' : approved ? 'verified' : 'awaiting_approval'
  f.item.version += 1
  f.mission.status = failed ? 'failed' : approved ? 'completed' : 'running'
  f.task.status = failed ? 'verification_failed' : approved ? 'completed' : 'awaiting_approval'
  const append = (type, payload, overrides = {}) => {
    const seq = f.replay.through + 1
    f.replay.events.push(event(c, seq, {
      type, payload, aggregate_id: replacementId, correlation_id: f.mission.id,
      room_id: f.mission.room_id, ...overrides,
    }))
    f.replay.through = seq
  }
  append('factory.verification_recovery_authorized', {
    factory_work_item_id: f.item.id, mission_id: f.mission.id, task_id: f.task.id,
    source_run_id: f.run.id, replacement_run_id: replacementId, mode: 'checkpoint_verification',
    reason: f.plan.reason, contract_revision_id: null,
  }, { aggregate_id: recoveryId, aggregate_type: 'factory_verification_recovery', actor_id: c.actor_id })
  append('run.verification_requested', {
    recovery_id: recoveryId, source_run_id: f.run.id, workspace_run_id: f.run.workspace_run_id,
    task_id: f.task.id, agent_id: f.agent.id, runner_id: c.runner_id,
    attempt: 1, max_attempts: 2, execution_mode: 'verification_only',
  }, { actor_id: c.actor_id })
  append('run.started', { execution_mode: 'verification_only', workspace: f.run.workspace_path,
    workspace_branch: f.run.workspace_branch, workspace_base_ref: 'HEAD', workspace_base_commit: EXPECTED.source_commit })
  append('runner.command_acknowledged', {
    command_id: id(111), command_kind: 'factory_verification_recovery', runner_id: c.runner_id,
  })
  append('run.verification_started', { check_count: f.plan.policy.checks.length })
  for (const [index, check] of f.plan.policy.checks.entries()) {
    const evidence = {
      id: id(120 + index), corp_id: c.corp_id, run_id: replacementId, task_id: f.task.id,
      check_index: index, kind: check.type,
      status: check.type === 'artifact' ? 'failed' : 'passed',
      summary: SENTINEL, payload: { bounded_stdout: SENTINEL },
    }
    f.state.snapshot.verification_evidence.push(evidence)
    append('run.verification_evidence', { evidence_id: evidence.id, check_index: index,
      kind: evidence.kind, status: evidence.status, summary: SENTINEL, payload: clone(evidence.payload) })
  }

  // These are opaque synthetic byte/hash fixtures, not a generated application or
  // a real Git bundle. No code in these content strings is executed.
  const fileContents = {
    'base.txt': 'base\n',
    'qa/issue195/index.html': '<!doctype html><title>In-memory unit data</title>',
    'qa/issue195/app.mjs': '// inert application byte fixture\n',
    'qa/issue195/app.test.mjs': '// inert test-source byte fixture\n',
  }
  const patch = Buffer.from('synthetic patch bytes')
  const bundle = Buffer.from('synthetic git bundle bytes')
  const document = {
    schema_version: 1, form: 'commit_branch', base_commit: EXPECTED.source_commit,
    head_commit: EXPECTED.source_commit, branch: f.run.workspace_branch,
    verification_sha256: 'd'.repeat(64),
    patch_base64: patch.toString('base64'), patch_sha256: sha(patch),
    git_bundle_base64: bundle.toString('base64'), git_bundle_sha256: sha(bundle),
    changes: Object.entries(fileContents).map(([file, content]) => {
      const bytes = Buffer.from(content)
      return { path: file, status: 'A', mode: '100644', bytes: bytes.length,
        sha256: sha(bytes), content_base64: bytes.toString('base64') }
    }),
  }
  const bytes = Buffer.from(JSON.stringify(document))
  const deliverable = {
    id: id(130), artifact_id: id(131), corp_id: c.corp_id, task_id: f.task.id,
    run_id: replacementId, form: 'commit_branch', bytes: bytes.length, sha256: sha(bytes),
    verification_sha256: 'd'.repeat(64), base_commit: EXPECTED.source_commit,
    head_commit: EXPECTED.source_commit, branch: f.run.workspace_branch,
    provenance_signature: SENTINEL, artifact_role: 'source_deliverable',
  }
  if (failed) {
    append(c.missing_artifact ? 'run.verification_failed' : 'run.failed', {
      error: c.missing_artifact ? 'required provider Artifact absent'
        : `verified deliverable export failed: verifier-only deliverable tree changed from preserved head ${EXPECTED.source_commit}; ${SENTINEL}`,
    })
  } else {
    replacement.deliverable_sha256 = deliverable.sha256
    f.state.snapshot.source_deliverables.push(deliverable)
    f.state.snapshot.verification_requests.push({
      run_id: replacementId, corp_id: c.corp_id, task_id: f.task.id,
      gate_type: 'independent_review', gate: clone(f.plan.policy.manual_gate),
      status: approved ? 'approved' : 'pending', decided_by: approved ? c.reviewer : null,
      decision_key: approved ? id(140) : null, decision_note: approved ? 'Offline approval fixture' : null,
    })
    append('run.deliverable', { artifact_id: deliverable.artifact_id, sha256: deliverable.sha256,
      verification_sha256: deliverable.verification_sha256 })
    append('run.verification_passed', { verification_sha256: deliverable.verification_sha256,
      deliverable_sha256: deliverable.sha256 })
    append('run.verification_waiting', { gate_type: 'independent_review',
      gate: clone(f.plan.policy.manual_gate), completion_summary: SENTINEL, verification_summary: SENTINEL })
    append('run.workspace_preserved', { workspace: f.run.workspace_path,
      workspace_fingerprint: f.proof.workspace_fingerprint, branch_deleted: false })
    if (approved) {
      append('verification.approved', { gate_type: 'independent_review', run_id: replacementId },
        { actor_id: c.reviewer, aggregate_type: 'verification' })
      append('factory.verified', { run_id: replacementId },
        { actor_id: c.reviewer, aggregate_type: 'factory_work_item', aggregate_id: f.item.id })
    }
  }
  if (failed) append('run.workspace_preserved', { workspace: f.run.workspace_path,
    workspace_fingerprint: f.proof.workspace_fingerprint, branch_deleted: false })
  f.state.snapshot.events = f.replay.events.slice(-2)
  return { ...f, original, replacement, recovery, context, deliverable, document, bytes, append }
}

test('synthetic VerifyRun evidence distinguishes awaiting independent review from accepted completion', () => {
  for (const approved of [false, true]) {
    const f = recoveryFixture(config(), { approved })
    const before = clone({ state: f.state, replay: f.replay, context: f.context })
    const result = exported('assessRecovery')(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity)
    assert.equal(result.verified, approved)
    assert.equal(result.run_id, f.replacement.id)
    assert.equal(result.source_run_id, f.run.id)
    assert.equal(result.recovery_id, f.recovery.id)
    assert.equal(result.original_counters_and_history_unchanged, true)
    for (const field of ['new_provider_sessions', 'new_provider_outputs', 'new_provider_usage_events',
      'model_token_allocation', 'model_cost_allocation', 'input_tokens', 'output_tokens', 'cost_microusd']) {
      assert.equal(result[field], 0)
    }
    assert.equal(result.provider_artifact_bytes_claimed, false)
    assert.equal(result.checks.length, 4)
    assert.ok(!JSON.stringify(result).includes(SENTINEL))
    assert.deepEqual({ state: f.state, replay: f.replay, context: f.context }, before)
  }
})

test('forced commit_branch export rejection is a named failure, never archive bypass or approval', () => {
  const f = recoveryFixture(config(), { exportFailure: true })
  assert.throws(() => exported('assessRecovery')(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity),
    { code: 'native_deliverable_export_rejects_uncommitted_checkpoint_app' })
  assert.equal(f.state.snapshot.verification_requests.length, 0)
  assert.equal(f.state.snapshot.source_deliverables.length, 0)
  assert.equal(f.task.contract.deliverable.form, 'commit_branch')
  assert.equal(f.state.snapshot.runs.length, 2)
  assert.equal(f.task.attempt_count, 1)
})

test('missing-required-provider-Artifact negative shares the native checks without inventing bytes', () => {
  const f = recoveryFixture(config({ missingArtifact: true }))
  const result = exported('assessRecovery')(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity)
  assert.equal(result.verified, false)
  assert.equal(result.missing_required_provider_artifact_rejected, true)
  assert.equal(result.provider_artifact_bytes_claimed, false)
  assert.deepEqual(result.checks.map((row) => row.status), ['passed', 'passed', 'passed', 'passed', 'failed'])
  assert.equal(f.state.snapshot.source_deliverables.length, 0)
  assert.equal(f.state.snapshot.verification_requests.length, 0)
})

for (const [label, mutate] of [
  ['original usage rewritten', (f) => { f.run.input_tokens = 0 }],
  ['original output history changed', (f) => {
    f.replay.events.find((e) => e.aggregate_id === f.run.id && e.type === 'run.output').payload.text = 'rewritten'
  }],
  ['provider-attempt counter reset', (f) => { f.task.attempt_count = 0 }],
  ['provider-attempt maximum changed', (f) => { f.task.max_attempts = 3 }],
  ['source incident changed', (f) => { f.incident.reason = 'rewritten' }],
  ['checkpoint authority absent', (f) => { f.context.checkpoint_verification = false }],
  ['different context source', (f) => { f.context.source_run_id = id(600) }],
  ['budget reset hidden in context', (f) => { f.context.remaining_mission_tokens = 6000 }],
  ['another native recovery', (f) => {
    f.state.snapshot.factory_verification_recoveries.push({ ...clone(f.recovery), id: id(601) })
  }],
  ['source-correction instead of verification', (f) => { f.recovery.mode = 'source_correction' }],
  ['recovery contract revision', (f) => { f.recovery.contract_revision_id = id(602) }],
  ['changed recovery reason', (f) => { f.recovery.reason += ' changed' }],
  ['weakened replacement policy', (f) => { f.recovery.replacement_verification_policy.checks.pop() }],
  ['new provider session', (f) => { f.replacement.provider_session_id = id(603) }],
  ['allocated model tokens', (f) => { f.replacement.budget_tokens_limit = 1 }],
  ['allocated model cost', (f) => { f.replacement.budget_cost_microusd_limit = 1 }],
  ['consumed input tokens', (f) => { f.replacement.input_tokens = 1 }],
  ['consumed output tokens', (f) => { f.replacement.output_tokens = 1 }],
  ['consumed model cost', (f) => { f.replacement.cost_microusd = 1 }],
  ['different workspace root run', (f) => { f.replacement.workspace_run_id = f.replacement.id }],
  ['replacement workspace not preserved', (f) => { f.replacement.workspace_disposition = 'active' }],
  ['replacement fingerprint changed', (f) => { f.replacement.workspace_fingerprint = '0'.repeat(64) }],
  ['invented provider artifact', (f) => { f.replacement.artifact_id = id(604) }],
  ['provider event outside snapshot window', (f) => { f.append('run.usage', { input_tokens: 1 }) }],
  ['native command acknowledgment missing', (f) => {
    f.replay.events = f.replay.events.filter((e) => !(e.aggregate_id === f.replacement.id &&
      e.type === 'runner.command_acknowledged'))
  }],
  ['missing native app check', (f) => { f.state.snapshot.verification_evidence.splice(3, 1) }],
  ['evidence kind mismatch', (f) => { f.state.snapshot.verification_evidence[1].kind = 'file' }],
  ['independent-review self-approval', (f) => { f.state.snapshot.verification_requests[0].decided_by = f.c.actor_id }],
  ['accepted run before gate decision', (f) => {
    f.state.snapshot.verification_requests[0].status = 'pending'
    f.state.snapshot.verification_requests[0].decided_by = null
  }],
  ['missing factory completion event', (f) => {
    f.replay.events = f.replay.events.filter((e) => e.type !== 'factory.verified')
  }],
  ['empty source deliverable', (f) => { f.deliverable.bytes = 0 }],
  ['wrong deliverable provenance', (f) => { f.deliverable.verification_sha256 = '0'.repeat(64) }],
]) {
  test(`assessRecovery refuses ${label}`, () => {
    const f = recoveryFixture(config(), { approved: true })
    mutate(f)
    assert.throws(() => exported('assessRecovery')(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity))
  })
}

test('download validation uses in-memory bytes and checks payload digests without claiming a browser test', () => {
  const f = recoveryFixture(config(), { approved: true })
  const result = exported('verifyDownload')(f.bytes, f.deliverable)
  assert.equal(result.sha256, sha(f.bytes))
  assert.equal(result.files.length, 4)
  assert.equal(result.browser_tested, false)
  assert.equal(result.published, false)
  assert.ok(!JSON.stringify(result).includes(SENTINEL))
  assert.throws(() => exported('verifyDownload')(Buffer.alloc(0), f.deliverable))
  assert.throws(() => exported('verifyDownload')(Buffer.from('tampered'), f.deliverable))
  for (const mutate of [
    (doc) => { doc.form = 'archive' },
    (doc) => { doc.base_commit = EXPECTED.code_commit },
    (doc) => { doc.changes = [] },
    (doc) => { doc.changes[1].path = '../outside.html' },
    (doc) => { doc.changes[1].content_base64 = Buffer.from('changed').toString('base64') },
    (doc) => { doc.changes[1].bytes += 1 },
    (doc) => { doc.changes[1] = clone(doc.changes[2]) },
    (doc) => { doc.changes.pop() },
    (doc) => { doc.git_bundle_base64 = '' },
    (doc) => { doc.patch_sha256 = '0'.repeat(64) },
  ]) {
    const document = clone(f.document)
    mutate(document)
    const bytes = Buffer.from(JSON.stringify(document))
    assert.throws(() => exported('verifyDownload')(bytes, { ...f.deliverable, bytes: bytes.length, sha256: sha(bytes) }))
  }
})

for (const target of ['http://localhost:18574', 'https://127.0.0.1:18574',
  'http://127.0.0.1:18574/', 'http://127.0.0.1:18575', 'http://127.0.0.1:8791',
  'http://127.0.0.1:18574/api', 'http://127.0.0.1:18574?x=1', 'http://example.invalid:18574',
  'http://user:password@127.0.0.1:18574', 'http://[::1]:18574']) {
  test(`configuration refuses target ${target}`, () => {
    const r = receipt()
    r.server_url = target
    assert.throws(() => config({}, r), { code: 'receipt_target_not_authorized' })
  })
}

test('absolute path and owned-output boundaries reject aliases, devices, UNC, ADS, and root reuse', () => {
  for (const field of ['receiptPath', 'outputDir', 'cli']) {
    for (const value of ['relative.exe', 'C:relative.exe', '\\\\host\\share\\file.exe',
      '\\\\.\\C:\\file.exe', 'C:\\dir\\..\\file.exe', 'C:\\dir\\.\\file.exe',
      'C:\\dir\\file.exe:stream', 'C:\\dir\\NUL.exe', 'C:\\dir\\file.exe ', 'C:\\']) {
      assert.throws(() => config({ [field]: value }), `${field}: ${value}`)
    }
  }
  for (const outputDir of [EXPECTED.runtime, `${EXPECTED.runtime}-neighbor\\output`,
    WIN.join(EXPECTED.runner_root, 'worktrees', 'output'), EXPECTED.source]) {
    assert.throws(() => config({ outputDir }))
  }
  assert.throws(() => config({ cli: WIN.join(EXPECTED.runtime, 'bin', 'candidate.cmd') }),
    { code: 'native_candidate_executable_required' })
  assert.throws(() => config({
    receiptPath: WIN.join(options().outputDir, 'checkpoint-verification-report.json'),
  }), { code: 'receipt_is_not_an_output' })
})

test('explicit independent reviewer, valid digest, and bounded numeric options are mandatory', () => {
  assert.throws(() => config({ reviewer: receipt().actor_id }), { code: 'reviewer_must_be_independent' })
  for (const reviewer of [undefined, '', 'owner', id(18).toUpperCase().replace('0000', 'GGGG')]) {
    assert.throws(() => config({ reviewer }))
  }
  for (const fixtureSha256 of [undefined, '', 'f'.repeat(63), 'g'.repeat(64)]) {
    assert.throws(() => config({ fixtureSha256 }))
  }
  for (const cliSha256 of ['', 'f'.repeat(63), 'g'.repeat(64)]) assert.throws(() => config({ cliSha256 }))
  for (const tokens of [0, -1, 4999, 5001, 6001, 5000.5, '5000', NaN, Infinity]) {
    assert.throws(() => config({ tokens }), { code: 'only_native_stop_or_suspend_budget' })
  }
  for (const overrides of [{ timeoutMs: 9999 }, { timeoutMs: 600001 }, { timeoutMs: NaN },
    { pollMs: 99 }, { pollMs: 2001 }, { settleMs: 1999 }, { settleMs: 30001 },
    { timeoutMs: 10000, settleMs: 3000 }]) assert.throws(() => config(overrides))
})

test('reports expose hashes and constrained identity, never receipt metadata or provider output', () => {
  const c = config()
  const report = driver.newReport(c, id(901))
  assert.equal(report.suite, 'checkpoint-verification-195')
  assert.equal(report.status, 'incomplete')
  assert.equal(report.passed, false)
  assert.equal(report.driver_id, id(901))
  assert.match(report.binding_sha256, /^[0-9a-f]{64}$/u)
  assert.deepEqual(report.intents, { create: null, recovery: null })
  assert.equal(report.pending_review, null)
  assert.ok(Object.values(report.identity).every((value) => value === null))
  assert.match(report.coverage.provider, /deterministic.*no real vendor/iu)
  assert.match(report.coverage.claim, /not finished issue148/iu)
  assert.ok(!JSON.stringify(report).includes(SENTINEL))
  assert.strictEqual(driver.validateSavedReport(report, c), report)
})

test('saved report cannot adopt changed authority, extra fields, or identities without prior intent', () => {
  const c = config()
  const base = driver.newReport(c, id(901))
  for (const overrides of [{ tokens: 6000 }, { reviewer: id(19) }, { fixtureSha256: 'd'.repeat(64) },
    { missingArtifact: true }, { cli: WIN.join(EXPECTED.runtime, 'bin', 'other.exe') }, { cliSha256: 'e'.repeat(64) }]) {
    assert.throws(() => driver.validateSavedReport(clone(base), config(overrides)),
      { code: 'saved_report_binding_mismatch' })
  }
  for (const mutate of [
    (r) => { r.suite = 'stopped-source-checkpoint-190' },
    (r) => { r.claim_token = SENTINEL },
    (r) => { r.identity.original_run_id = id(110) },
    (r) => { r.identity.replacement_run_id = id(111) },
    (r) => { r.intents.create = { at: 1, sha256: 'a'.repeat(64) } },
    (r) => { r.intents.review = null },
    (r) => { r.identity.unrecorded_run_id = id(112) },
  ]) {
    const report = clone(base)
    mutate(report)
    assert.throws(() => driver.validateSavedReport(report, c))
  }
  assert.doesNotThrow(() => driver.validateSavedReport(clone(base), config({ timeoutMs: 200_000 })))
})

test('baseline and journal never consult the bounded snapshot.events window', () => {
  const c = config()
  const { state, replay } = history(c)
  Object.defineProperty(state.snapshot, 'events', { get() { throw new Error('snapshot.events must not be read') } })
  const baseline = driver.captureBaseline(state, replay, c)
  assert.equal(baseline.journal.event_count, 301)
  assert.equal(baseline.journal.through, 301)
  assert.doesNotThrow(() => driver.assertBaseline(baseline, state, replay, c))
  const journal = driver.journalView(replay, c)
  assert.equal(journal.events.length, 301)
  assert.equal(journal.events[0].id, replay.events[0].id)
  assert.equal(journal.events[0].sha256, hashJson(replay.events[0]))
  assert.equal(journal.events[0].payload_sha256, hashJson(replay.events[0].payload))
  assert.deepEqual(Object.keys(journal.events[0]).sort(),
    ['id', 'seq', 'type', 'aggregate_id', 'correlation_id', 'room_id', 'actor_id', 'sha256', 'payload_sha256', 'payload'].sort())
  assert.ok(!JSON.stringify({ baseline, journal }).includes(SENTINEL))
  const changed = clone(replay)
  changed.events[0].payload.text = 'changed before the snapshot window'
  assert.throws(() => driver.assertBaseline(baseline, state, changed, c),
    { code: 'prior_visible_journal_prefix_changed' })
})

for (const table of ['missions', 'tasks', 'runs', ...EXTRA]) {
  test(`baseline preserves every pre-existing ${table} record`, () => {
    const c = config()
    const { state, replay } = history(c, 1)
    const baseline = driver.captureBaseline(state, replay, c)
    const changed = clone(state)
    changed.snapshot[table][0].status = 'rewritten'
    assert.throws(() => driver.assertBaseline(baseline, changed, replay, c))
    const removed = clone(state)
    removed.snapshot[table] = []
    assert.throws(() => driver.assertBaseline(baseline, removed, replay, c))
  })
}

test('baseline permits additive rows and events while forbidding live work and missing history tables', () => {
  const c = config()
  const { state, replay } = history(c, 1)
  const baseline = driver.captureBaseline(state, replay, c)
  const added = clone(state)
  added.snapshot.missions.push({ id: id(70), corp_id: c.corp_id, status: 'failed' })
  const extended = { events: [...replay.events, event(c, 4)], through: 4 }
  assert.doesNotThrow(() => driver.assertBaseline(baseline, added, extended, c))
  const busy = clone(state)
  busy.snapshot.runs[0].status = 'running'
  assert.throws(() => driver.captureBaseline(busy, replay, c), { code: 'existing_live_runs_block_new_suite' })
  for (const table of EXTRA) {
    const missing = clone(state)
    delete missing.snapshot[table]
    assert.throws(() => driver.captureBaseline(missing, replay, c))
  }
})

test('journal requires exact Ready watermark, ordered unique same-Corp events, and bounded count', () => {
  const c = config()
  const a = event(c, 2)
  const b = event(c, 9)
  assert.equal(driver.journalView({ events: [a, b], through: 9 }, c).event_count, 2)
  for (const replay of [
    { events: [a, b], through: 10 }, { events: [b, a], through: 2 },
    { events: [a, a], through: 2 }, { events: [{ ...a, corp_id: id(999) }], through: 2 },
    { events: [{ ...a, type: SENTINEL }], through: 2 },
    { events: Array(20_001).fill(a), through: 2 },
  ]) assert.throws(() => driver.journalView(replay, c))
})

test('buildPlan is deterministic, fresh, output-owned, and mirrors one ready synthetic issue', () => {
  const c = config()
  const before = clone(c)
  const buildPlan = exported('buildPlan')
  const plan = buildPlan(c, id(901))
  assert.deepEqual(buildPlan(c, id(901)), plan)
  assert.deepEqual(c, before)
  const other = buildPlan(c, id(902))
  assert.notEqual(plan.item_id, other.item_id)
  assert.notEqual(plan.issue_number, other.issue_number)
  const otherOutput = buildPlan(config({ outputDir: WIN.join(EXPECTED.runtime, 'other-owned-output') }), id(902))
  assert.notEqual(plan.state_path, otherOutput.state_path)
  assert.notEqual(plan.policy_path, otherOutput.policy_path)
  assertOwnedFile(plan.state_path, c.output_dir)
  assertOwnedFile(plan.policy_path, c.output_dir)
  assert.notEqual(plan.state_path, plan.policy_path)
  assert.ok(Number.isSafeInteger(plan.issue_number) && plan.issue_number > 0)
  assert.ok(Number.isSafeInteger(plan.project_number) && plan.project_number > 0)
  assert.equal(plan.github_state.repository, EXPECTED.repository)
  assert.equal(plan.github_state.project.number, plan.project_number)
  assert.equal(plan.github_state.items.length, 1)
  assert.deepEqual(Object.keys(plan.github_state.issues), [String(plan.issue_number)])
  assert.deepEqual(plan.github_state.issues[String(plan.issue_number)], plan.issue)
  const item = plan.github_state.items[0]
  assert.equal(item.id, plan.item_id)
  assert.equal(item.status, 'Todo')
  assert.equal(item.content.type, 'Issue')
  assert.equal(item.content.repository, EXPECTED.repository)
  assert.equal(item.content.number, plan.issue_number)
  for (const key of ['title', 'body', 'url']) assert.equal(item.content[key], plan.issue[key])
  assert.equal(plan.issue.state, 'OPEN')
  assert.ok(plan.issue.labels.some(({ name }) => name === 'factory:ready'))
  assert.match(`${plan.issue.title}\n${plan.issue.body}`, /\[budget-stream\]/u)
  assert.match(`${plan.issue.title}\n${plan.issue.body}`, /\[checkpoint-app\]/u)
  assert.deepEqual(plan.policy, exported('verifierPolicy')(c))
  assert.ok(typeof plan.reason === 'string' && plan.reason.trim().length > 0)
  assert.ok(!JSON.stringify(plan).includes(SENTINEL))
})

test('factory argv remains pinned and recovery only appends the exact checkpoint mode and recorded reason', () => {
  const c = config()
  const plan = exported('buildPlan')(c, id(901))
  const before = clone({ c, plan })
  const args = exported('factoryArgs')(c, plan, false)
  assert.deepEqual(args.slice(0, 5), ['--server', EXPECTED.server, 'factory', c.corp_id, c.actor_id])
  const value = (name) => {
    assert.equal(args.filter((arg) => arg === name).length, 1, `${name} occurs exactly once`)
    return args[args.indexOf(name) + 1]
  }
  assert.equal(value('--repository'), EXPECTED.repository)
  assert.equal(WIN.normalize(value('--source-repository-path')), EXPECTED.source)
  assert.equal(value('--source-base-ref'), 'HEAD')
  assert.equal(value('--adapter'), 'codex')
  assert.equal(value('--allow-adapter'), 'codex')
  assert.equal(value('--strategy'), 'single')
  assert.equal(value('--budget-tokens'), '5000')
  assert.ok(Number(value('--budget-cost-microusd')) > 0)
  assert.equal(value('--issue'), String(plan.issue_number))
  assert.equal(value('--project-number'), String(plan.project_number))
  assert.equal(value('--verification-policy-file'), plan.policy_path)
  assert.equal(value('--write-scope'), '**')
  assert.ok([c.fake_github_cli, process.execPath].includes(value('--github-cli')))
  for (const flag of ['--max-attempts', '--reset', '--bootstrap', '--dry-run', '--access-token',
    '--verification-recovery', '--model', '--reasoning-effort']) assert.ok(!args.includes(flag))
  assert.deepEqual(exported('factoryArgs')(c, plan, true),
    [...args, '--verification-recovery', 'checkpoint-verification', '--verification-recovery-reason', plan.reason])
  assert.deepEqual({ c, plan }, before)
  const suspend = config({ tokens: 6000 })
  const suspendArgs = exported('factoryArgs')(suspend, exported('buildPlan')(suspend, id(902)), false)
  assert.equal(suspendArgs[suspendArgs.indexOf('--budget-tokens') + 1], '6000')
})

function runHashAssertion(script, bytes) {
  let reads = 0
  const fakeProcess = { exitCode: 0, exit(code = 0) { this.exitCode = code; throw new Error(`exit:${code}`) } }
  try {
    runInNewContext(script, {
      Buffer, process: fakeProcess, console: { log() {}, error() {} },
      require(name) {
        if (name === 'node:crypto' || name === 'crypto') return { createHash }
        if (name === 'node:assert/strict') return assert
        if (name === 'node:fs' || name === 'fs') return {
          readFileSync(file, encoding) {
            assert.equal(file, 'base.txt')
            reads += 1
            if (bytes === null) throw new Error('synthetic missing file')
            return encoding ? bytes.toString(encoding) : Buffer.from(bytes)
          },
        }
        throw new Error(`offline test rejects module ${name}`)
      },
    }, { timeout: 1000 })
    return { passed: fakeProcess.exitCode === 0, reads }
  } catch {
    return { passed: false, reads }
  }
}

test('persisted app, base-file, exact hash command and independent-review checks cannot be replaced by Artifact', () => {
  const c = config()
  const policy = exported('verifierPolicy')(c)
  assert.deepEqual(policy.manual_gate,
    { type: 'independent_review', roles: ['owner', 'admin', 'manager', 'member'], exclude_requester: true })
  assert.equal(policy.checks.length, 4)
  assert.ok(policy.checks.some((check) =>
    check.type === 'file' && check.path === 'base.txt' && check.min_bytes === 5))
  assert.ok(policy.checks.some((check) =>
    check.type === 'file' && check.path === 'qa/issue195/index.html' && check.min_bytes > 0))
  const app = policy.checks.find((check) => check.type === 'command' && check.args[0] === '--test')
  assert.ok(app, 'native Node app tests must be a persisted Command verifier')
  assert.deepEqual(app.args, ['--test', 'qa/issue195/app.test.mjs'])
  const exact = policy.checks.find((check) => check.type === 'command' && check.args[0] === '-e')
  assert.ok(exact, 'base bytes must have a persisted exact-hash command')
  assert.equal(exact.args.length, 2)
  assert.ok(exact.args[1].includes(BASE_SHA))
  for (const check of [app, exact]) {
    assert.equal(check.program, 'node')
    assert.ok(Number.isSafeInteger(check.timeout_ms) && check.timeout_ms > 0 && check.timeout_ms <= 60_000)
  }
  assert.deepEqual(runHashAssertion(exact.args[1], Buffer.from('base\n')), { passed: true, reads: 1 })
  for (const bytes of [Buffer.from('base\r\n'), Buffer.from('\ufeffbase\n'), Buffer.from('base\nextra'),
    Buffer.from('BASE\n'), Buffer.alloc(0), null]) {
    assert.equal(runHashAssertion(exact.args[1], bytes).passed, false)
  }
  const negative = exported('verifierPolicy')(config({ missingArtifact: true }))
  assert.deepEqual(negative.manual_gate, policy.manual_gate)
  assert.deepEqual(negative.checks.filter((check) => check.type !== 'artifact'), policy.checks)
  assert.equal(negative.checks.filter((check) => check.type === 'artifact').length, 1)
})

test('checkpoint digests use Rust field order, preserve app policy, and retain forced commit_branch authority', () => {
  const policy = exported('verifierPolicy')(config())
  const checks = policy.checks.map((check) => check.type === 'file'
    ? { type: check.type, path: check.path, min_bytes: check.min_bytes }
    : { type: check.type, program: check.program, args: check.args, timeout_ms: check.timeout_ms })
  const rust = JSON.stringify({
    checks, manual_gate: { type: policy.manual_gate.type, roles: policy.manual_gate.roles, exclude_requester: true },
  })
  const expected = {
    verification_policy_sha256: sha(rust),
    write_scope_sha256: sha(JSON.stringify(['**'])),
    deliverable_policy_sha256: sha(JSON.stringify({
      form: 'commit_branch', commit_after_verification: true, paths: [],
    })),
  }
  assert.deepEqual(exported('checkpointPolicyDigests')(policy), expected)
  const reordered = JSON.parse(canonical(policy))
  assert.deepEqual(exported('checkpointPolicyDigests')(reordered), expected)
})

test('invokeFactory uses only injected child I/O, a bounded timeout, and a non-inherited environment', async () => {
  const c = config()
  const plan = exported('buildPlan')(c, id(901))
  const calls = []
  const fakeExecute = (file, args, settings, done) => {
    calls.push({ file, args, settings })
    done(null, JSON.stringify({ factory_work_item_id: id(100), mission_id: id(101), launch: { run_id: id(102) } }))
  }
  const result = await driver.invokeFactory(c, plan, false, 120_000, fakeExecute)
  assert.equal(result.mission_id, id(101))
  assert.equal(calls.length, 1)
  const call = calls[0]
  assert.equal(call.file, c.cli)
  assert.deepEqual(call.args, exported('factoryArgs')(c, plan, false))
  assert.equal(call.settings.cwd, c.output_dir)
  assert.equal(call.settings.shell, false)
  assert.equal(call.settings.windowsHide, true)
  assert.equal(call.settings.timeout, 60_000)
  assert.equal(call.settings.maxBuffer, 2 * 1024 * 1024)
  assert.equal(call.settings.env.ECORP_FAKE_GITHUB_STATE, plan.state_path)
  assert.equal(call.settings.env.PATH, `${path.dirname(process.execPath)};${WIN.dirname(EXPECTED.git)}`)
  assert.ok(Object.keys(call.settings.env).every((key) =>
    ['PATH', 'ECORP_FAKE_GITHUB_STATE', 'ECORP_GITHUB_CLI_PREFIX_ARGS_JSON'].includes(key)))
  const githubBin = call.args[call.args.indexOf('--github-cli') + 1]
  if (githubBin === process.execPath) {
    assert.deepEqual(JSON.parse(call.settings.env.ECORP_GITHUB_CLI_PREFIX_ARGS_JSON), [c.fake_github_cli])
  }
})

test('invokeFactory never retries an uncertain execution or logs raw stdout/stderr', async (t) => {
  const c = config()
  const plan = exported('buildPlan')(c, id(901))
  const printed = []
  for (const method of ['log', 'warn', 'error']) t.mock.method(console, method, (...args) => printed.push(args))
  let calls = 0
  await assert.rejects(driver.invokeFactory(c, plan, true, 1000, (_file, _args, _settings, done) => {
    calls += 1
    done(Object.assign(new Error(SENTINEL), { stdout: SENTINEL, stderr: SENTINEL }),
      JSON.stringify({ claim_token: SENTINEL }))
  }), { code: 'factory_cli_failed_or_uncertain_do_not_retry' })
  assert.equal(calls, 1)
  assert.deepEqual(printed, [])
  for (const stdout of [SENTINEL, 'null', '42', JSON.stringify(SENTINEL), '[]']) {
    let attempts = 0
    await assert.rejects(driver.invokeFactory(c, plan, false, 1000, (_file, _args, _settings, done) => {
      attempts += 1
      done(null, stdout)
    }), { code: 'factory_response_invalid_do_not_retry' })
    assert.equal(attempts, 1)
  }
})

function originalFixture(c = config(), driverId = id(901)) {
  const plan = exported('buildPlan')(c, driverId)
  const state = emptyState(c)
  const missionId = id(101)
  const taskId = id(102)
  const agentId = id(103)
  const runId = id(105)
  const status = c.tokens === 5000 ? 'failed' : 'cancelled'
  const stage = c.tokens === 5000 ? 'stop' : 'suspend'
  const deliverable = { form: 'commit_branch', commit_after_verification: true, paths: [] }
  const item = {
    id: id(100), corp_id: c.corp_id, source_kind: 'github_project',
    source_project_owner: 'shyamsridhar123', source_project_number: plan.project_number,
    source_project_item_id: plan.item_id, source_repository_owner: 'shyamsridhar123',
    source_repository_name: 'ecorp-enterprise-lab', source_issue_number: plan.issue_number,
    source_issue_node_id: plan.issue.id, source_issue_url: plan.issue.url,
    source_title: plan.issue.title, source_revision: plan.issue.updatedAt,
    claim_owner_id: c.actor_id, mission_id: missionId, version: 3,
    state: 'blocked', failure_detail: SENTINEL,
    policy: {
      source_base_ref: 'HEAD', source_base_commit: EXPECTED.source_commit,
      budget_tokens: c.tokens, budget_cost_microusd: 1_000_000, auto_merge: false,
      verification_policy: clone(plan.policy), write_scope: ['**'], deliverable_form: 'commit_branch',
      repository_allowlist: [EXPECTED.repository], adapter_allowlist: ['codex'],
      verification_required: true, secret_ids: [],
    },
  }
  const mission = {
    id: missionId, corp_id: c.corp_id, requested_by: c.actor_id, room_id: id(20),
    title: `GitHub #${plan.issue_number}: ${plan.issue.title}`, description: plan.issue.body,
    strategy: 'single', specification_version: 1, status,
    budget_tokens: c.tokens, original_budget_tokens: c.tokens,
    budget_cost_microusd: 1_000_000, original_budget_cost_microusd: 1_000_000,
  }
  const task = {
    id: taskId, corp_id: c.corp_id, mission_id: missionId, assigned_agent_id: agentId,
    required_adapter: 'codex', max_attempts: 2, attempt_count: 1, contract_version: 1,
    depth: 0, depends_on: [], status, verification_status: 'pending',
    verification_policy: clone(plan.policy),
    contract: {
      objective: plan.issue.body, source_repository: EXPECTED.repository, source_base_ref: 'HEAD',
      source_base_commit: EXPECTED.source_commit, budget_tokens: c.tokens,
      budget_cost_microusd: 1_000_000, model: null, reasoning_effort: null,
      write_scope: ['**'], secret_refs: [], deliverable,
    },
  }
  const agent = {
    id: agentId, corp_id: c.corp_id, actor_id: id(104), mission_id: missionId,
    adapter: 'codex', current_run_id: null, status: 'idle', retired_at: null,
  }
  const branch = `crony/task-${taskId.replaceAll('-', '')}/run-${runId.replaceAll('-', '')}`
  const workspace = path.join(c.runner_root, 'worktrees', taskId.replaceAll('-', ''), runId.replaceAll('-', ''))
  const run = {
    id: runId, corp_id: c.corp_id, task_id: taskId, agent_id: agentId, runner_id: c.runner_id,
    execution_mode: 'provider', status, breaker_stage: stage, workspace_disposition: 'preserved',
    workspace_run_id: runId, resumed_from_run_id: null, provider_session_id: id(106),
    source_repository: EXPECTED.repository, source_base_ref: 'HEAD', source_base_commit: EXPECTED.source_commit,
    workspace_path: workspace, workspace_branch: branch, workspace_base_ref: 'HEAD',
    workspace_base_commit: EXPECTED.source_commit, workspace_fingerprint: 'b'.repeat(64),
    model: null, reasoning_effort: null, budget_tokens_limit: c.tokens,
    budget_cost_microusd_limit: 1_000_000, input_tokens: 6000, output_tokens: 0, cost_microusd: 0,
    no_progress_events: 0, repeated_tool_count: 0, verification_status: 'pending',
    artifact_id: null, artifact_uri: null, artifact_sha256: null, artifact_signature: null,
    artifact_media_type: null, verification_sha256: null, deliverable_sha256: null,
    verification_summary: null, summary: SENTINEL,
  }
  const proof = {
    schema_version: 1, corp_id: c.corp_id, mission_id: missionId, task_id: taskId,
    run_id: runId, workspace_run_id: runId, agent_id: agentId, runner_id: c.runner_id,
    source_repository: EXPECTED.repository, source_base_ref: 'HEAD', source_base_commit: EXPECTED.source_commit,
    workspace_base_commit: EXPECTED.source_commit, branch, head_commit: EXPECTED.source_commit,
    workspace_fingerprint: run.workspace_fingerprint, ...exported('checkpointPolicyDigests')(task.verification_policy),
  }
  const commonWorkspace = {
    workspace, workspace_branch: branch, workspace_base_ref: 'HEAD', workspace_base_commit: EXPECTED.source_commit,
  }
  const input = { metric: 'run_tokens', used: 6000, limit: c.tokens }
  const commandId = id(107)
  const entries = [
    ['run.requested', { mission_launch: true, task_id: taskId, agent_id: agentId,
      runner_id: c.runner_id, attempt: 1, max_attempts: 2 }],
    ['run.started', { ...commonWorkspace, adapter: 'codex', mission_id: missionId, task_id: taskId, room_id: id(20) }],
    ['run.session', { session_id: run.provider_session_id }],
    ['run.output', { text: SENTINEL }],
    ['run.usage', { input_tokens: 3000, output_tokens: 0, cost_microusd: 0 }],
    ['run.usage', { input_tokens: 3000, output_tokens: 0, cost_microusd: 0 }],
    ['run.breaker_transition', { stage, command_id: commandId, input }],
    ['runner.command_acknowledged', { command_id: commandId, command_kind: 'circuit_breaker', runner_id: c.runner_id }],
    ['run.session_terminated', { adapter: 'codex', provider_process_alive: false, outcome: status }],
    ['run.workspace_preserved', { ...commonWorkspace, source_checkpoint: proof,
      workspace_fingerprint: proof.workspace_fingerprint, head_commit: proof.head_commit, branch_deleted: false }],
    [`run.${status}`, { error: SENTINEL }],
  ]
  const events = entries.map(([type, payload], index) => event(c, index + 1, {
    type, payload, aggregate_id: runId, correlation_id: missionId, room_id: id(20),
    actor_id: type === 'run.requested' ? c.actor_id : null,
  }))
  const incident = { id: id(108), corp_id: c.corp_id, mission_id: missionId, task_id: taskId,
    run_id: runId, stage, input, reason: SENTINEL }
  Object.assign(state.snapshot, {
    factory_work_items: [item], missions: [mission], tasks: [task], runs: [run],
    agents: [agent], circuit_breaker_incidents: [incident], events: events.slice(-2),
  })
  return { c, plan, state, item, mission, task, agent, run, proof, incident,
    replay: { events, through: events.at(-1).seq }, identity: driver.newReport(c, driverId).identity }
}

for (const tokens of [5000, 6000]) {
  test(`assessOriginal binds native ${tokens === 5000 ? 'stop' : 'suspend'} from full replay, not snapshot.events`, () => {
    const fixture = originalFixture(config({ tokens }))
    const { c, plan, state, replay, identity, run, task, mission, item } = fixture
    Object.defineProperty(state.snapshot, 'events', { get() { throw new Error('bounded snapshot is not journal proof') } })
    const observed = exported('assessOriginal')(state, replay, c, plan, identity)
    assert.deepEqual(identity, {
      work_item_id: item.id, mission_id: mission.id, task_id: task.id, original_run_id: run.id,
      replacement_run_id: null, recovery_id: null,
    })
    assert.equal(observed.run_sha256, hashJson(run))
    assert.equal(observed.run_events_sha256, hashJson(replay.events))
    assert.equal(observed.run_event_count, replay.events.length)
    assert.equal(observed.provider_session_sha256, sha(run.provider_session_id))
    assert.equal(observed.accepted_provider_artifacts, 0)
    assert.deepEqual(observed.counters, {
      attempt_count: 1, max_attempts: 2, input_tokens: 6000, output_tokens: 0,
      cost_microusd: 0, no_progress_events: 0, repeated_tool_count: 0,
    })
    assert.ok(!JSON.stringify(observed).includes(SENTINEL))
    assert.ok(!JSON.stringify(observed).includes(run.provider_session_id))
    assert.throws(() => exported('assessOriginal')(state,
      { events: replay.events.slice(-2), through: replay.through }, c, plan, clone(identity)))
  })
}

for (const [label, mutate] of [
  ['provider attempt increment', (f) => { f.task.attempt_count = 2 }],
  ['attempt ceiling revision', (f) => { f.task.max_attempts = 3 }],
  ['current budget revision', (f) => { f.mission.budget_tokens += 1 }],
  ['original budget rewrite', (f) => { f.mission.original_budget_tokens += 1 }],
  ['persisted hash verifier removal', (f) => { f.task.verification_policy.checks.splice(1, 1) }],
  ['manual gate weakening', (f) => { f.task.verification_policy.manual_gate.exclude_requester = false }],
  ['contract source replacement', (f) => { f.task.contract.source_base_commit = EXPECTED.code_commit }],
  ['write scope replacement', (f) => { f.task.contract.write_scope = ['base.txt'] }],
  ['archive bypass of forced commit_branch', (f) => { f.task.contract.deliverable.form = 'archive' }],
  ['provider secret authority', (f) => { f.task.contract.secret_refs.push({ secret_id: id(500) }) }],
  ['extra provider run', (f) => { f.state.snapshot.runs.push({ ...clone(f.run), id: id(501) }) }],
  ['extra task', (f) => { f.state.snapshot.tasks.push({ ...clone(f.task), id: id(502) }) }],
  ['changed factory owner', (f) => { f.item.claim_owner_id = f.c.reviewer }],
  ['changed issue revision', (f) => { f.item.source_revision = '2026-09-09T00:00:00Z' }],
  ['wrong assigned runner', (f) => { f.run.runner_id = 'unowned-runner' }],
  ['producer reviewing itself', (f) => { f.agent.actor_id = f.c.reviewer }],
  ['source-checkout execution', (f) => { f.run.workspace_path = EXPECTED.source }],
  ['quarantined checkpoint', (f) => { f.run.workspace_disposition = 'quarantined' }],
  ['loop-counter change', (f) => { f.run.repeated_tool_count = 1 }],
  ['rewritten usage', (f) => { f.run.input_tokens = 5999 }],
  ['invented provider artifact', (f) => { f.run.artifact_id = id(503) }],
  ['checkpoint missing', (f) => {
    delete f.replay.events.find((e) => e.type === 'run.workspace_preserved').payload.source_checkpoint
  }],
  ['checkpoint policy digest changed', (f) => { f.proof.verification_policy_sha256 = '0'.repeat(64) }],
  ['provider still alive', (f) => {
    f.replay.events.find((e) => e.type === 'run.session_terminated').payload.provider_process_alive = true
  }],
  ['budget command not acknowledged', (f) => {
    f.replay.events.find((e) => e.type === 'runner.command_acknowledged').payload.command_id = id(504)
  }],
  ['budget incident missing', (f) => { f.state.snapshot.circuit_breaker_incidents = [] }],
  ['previously recorded run missing', (f) => { f.identity.original_run_id = id(505) }],
  ['accepted provider progress hidden before snapshot window', (f) => {
    f.replay.events.find((e) => e.type === 'run.output').type = 'run.artifact'
  }],
]) {
  test(`assessOriginal rejects ${label} without accepting or replacing work`, () => {
    const f = originalFixture()
    mutate(f)
    assert.throws(() => exported('assessOriginal')(f.state, f.replay, f.c, f.plan, f.identity))
  })
}

function memoryHarness(c = config({ timeoutMs: 15_000, pollMs: 250, settleMs: 2000 }), behavior = {}) {
  const driverId = id(901)
  const plan = exported('buildPlan')(c, driverId)
  const old = history(c, 1)
  const original = originalFixture(c, driverId)
  const recovered = recoveryFixture(c, { exportFailure: behavior.exportFailure === true })
  const approved = recoveryFixture(c, { approved: true })
  const mutations = []
  const saves = []
  const downloads = []
  const reads = { snapshot: 0, replay: 0, context: 0, base: 0, download: 0, checkInputs: 0 }
  let clock = Date.parse('2026-09-08T16:00:00.000Z')
  let phase = behavior.initial ?? 'empty'
  let failedSave = false
  const fixture = () => phase === 'empty' ? null : phase === 'original' ? original
    : phase === 'approved' ? approved : recovered
  const snapshot = () => {
    const state = clone(old.state)
    const current = fixture()
    if (current) {
      for (const table of ['missions', 'tasks', 'runs', 'agents', ...EXTRA]) {
        state.snapshot[table].push(...clone(current.state.snapshot[table]))
      }
    }
    behavior.onSnapshot?.(state, phase)
    // The suite must use replay, not silently infer history/absence from this window.
    Object.defineProperty(state.snapshot, 'events', {
      configurable: true, get() { throw new Error('snapshot.events is not an event journal') },
    })
    return state
  }
  const replay = () => {
    const events = clone(old.replay.events)
    for (const entry of fixture()?.replay.events ?? []) {
      events.push({ ...clone(entry), id: id(20_000 + entry.seq), seq: entry.seq + old.replay.through })
    }
    return { events, through: events.at(-1)?.seq ?? 0 }
  }
  const lastSave = () => saves.at(-1)
  const assertIntent = (name, request) => {
    assert.ok(lastSave()?.baseline, 'baseline must be durably recorded before a native mutation')
    assert.ok(lastSave().journal, 'full replay watermark must precede the native mutation')
    assert.equal(lastSave().intents[name]?.sha256, hashJson(request), 'exact intent must be saved before the effect')
    assert.equal(mutations.filter((entry) => entry.operation === name).length, 0, 'native effects must not be retried')
  }
  const io = {
    now: () => clock,
    sleep: async (ms) => {
      assert.ok(Number.isFinite(ms) && ms > 0 && ms <= c.timeout_ms)
      clock += ms
    },
    snapshot: async () => { reads.snapshot += 1; return snapshot() },
    replay: async () => { reads.replay += 1; return replay() },
    save: async (report) => {
      assert.ok(!JSON.stringify(report).includes(SENTINEL), 'reports must withhold raw error/output/token values')
      if (behavior.failIntentSave && report.intents.create && !failedSave) {
        failedSave = true
        throw new Error(SENTINEL)
      }
      saves.push(clone(report))
    },
    checkInputs: async () => {
      reads.checkInputs += 1
      if (reads.checkInputs === behavior.failInputCheck) throw new Error(SENTINEL)
    },
    factory: async (recovery, timeoutMs) => {
      assert.ok(timeoutMs > 0 && timeoutMs <= c.timeout_ms)
      const operation = recovery ? 'recovery' : 'create'
      const args = exported('factoryArgs')(c, plan, recovery)
      const request = recovery ? {
        args, original_run_id: original.run.id, source_checkpoint: lastSave().original.source_checkpoint,
      } : args
      assertIntent(operation, request)
      mutations.push({ operation, args: clone(args) })
      if (behavior.uncertain === `${operation}_unknown`) throw new Error(SENTINEL)
      phase = recovery ? 'recovered' : 'original'
      if (behavior.uncertain === `${operation}_lost`) throw new Error(SENTINEL)
      return {
        factory_work_item_id: original.item.id, mission_id: original.mission.id,
        claim_token: SENTINEL,
        launch: recovery ? { run_id: recovered.replacement.id, recovery_id: recovered.recovery.id,
          recovery_mode: 'checkpoint_verification', verification_recovery: true, assignment_token: SENTINEL }
          : { run_id: original.run.id, assignment_token: SENTINEL },
      }
    },
    context: async (workItemId) => {
      reads.context += 1
      assert.equal(workItemId, original.item.id)
      assert.equal(phase, 'original', 'do not query failed-task admission after a recovery already exists')
      return { ...clone(recovered.context), work_item: clone(original.item), recoveries: [] }
    },
    readBase: async (identity) => {
      reads.base += 1
      assert.deepEqual(identity, { task_id: original.task.id, run_id: original.run.id })
      return {
        path: path.join(original.run.workspace_path, 'base.txt'), bytes: 5,
        sha256: behavior.badBase ? '0'.repeat(64) : BASE_SHA,
      }
    },
    download: async (deliverable) => {
      reads.download += 1
      assert.equal(deliverable.artifact_id, approved.deliverable.artifact_id)
      assert.equal(deliverable.sha256, approved.deliverable.sha256)
      assert.equal(phase, 'approved')
      return behavior.badDownload ? Buffer.from('tampered') : Buffer.from(approved.bytes)
    },
    saveDownload: async (bytes) => { downloads.push(Buffer.from(bytes)) },
  }
  return {
    c, plan, io, reads, saves, mutations, downloads, snapshot, replay,
    report: driver.newReport(c, driverId),
    original, recovered, approved,
    setPhase(value) { phase = value },
    get phase() { return phase },
    get elapsed() { return clock - Date.parse('2026-09-08T16:00:00.000Z') },
  }
}

test('suite pauses for native browser review by default and only observes external approval on continuation', async () => {
  const h = memoryHarness()
  const paused = await exported('executeSuite')(h.c, h.report, h.io)
  assert.equal(paused.status, 'awaiting_browser_review')
  assert.equal(paused.passed, false)
  assert.equal(paused.replacement.verified, false)
  assert.equal(paused.pending_review.run_id, h.recovered.replacement.id)
  assert.equal(paused.pending_review.recovery_id, h.recovered.recovery.id)
  assert.equal(paused.pending_review.work_item_id, h.original.item.id)
  assert.equal(paused.pending_review.mission_id, h.original.mission.id)
  assert.equal(paused.pending_review.task_id, h.original.task.id)
  assert.equal(paused.pending_review.reviewer_actor_id, h.c.reviewer)
  assert.equal(paused.pending_review.gate_type, 'independent_review')
  assert.deepEqual(paused.pending_review.evidence_ids, paused.replacement.checks.map((check) => check.id))
  assert.equal(paused.pending_review.verification_sha256, paused.replacement.deliverable.verification_sha256)
  assert.equal(paused.download, null)
  assert.equal(h.reads.download, 0)
  assert.equal(Object.hasOwn(h.io, 'decide'), false)
  assert.deepEqual(Object.keys(paused.intents).sort(), ['create', 'recovery'])
  const pendingReview = clone(paused.pending_review)
  const intents = clone(paused.intents)
  await exported('executeSuite')(h.c, h.report, h.io, { continuation: true })
  assert.equal(h.report.status, 'awaiting_browser_review')
  assert.equal(h.report.passed, false)
  assert.deepEqual(h.report.pending_review, pendingReview)
  assert.deepEqual(h.report.intents, intents)
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
  // Simulate a distinct parent's native browser decision by changing MEMORY only.
  // No test or production decision API exists here.
  h.setPhase('approved')
  const result = await exported('executeSuite')(h.c, h.report, h.io, { continuation: true })
  assert.equal(result.passed, true)
  assert.equal(result.status, 'verified_and_downloaded')
  assert.equal(result.pending_review.status, 'approved')
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
  assert.deepEqual(result.intents, intents)
  assert.equal(h.reads.checkInputs, 2)
  assert.equal(h.reads.context, 1)
  assert.equal(h.reads.download, 1)
  assert.equal(h.downloads.length, 1)
  assert.equal(result.replacement.original_counters_and_history_unchanged, true)
  assert.equal(result.download.browser_tested, false)
  assert.equal(result.download.published, false)
  assert.equal(result.journal.events.length, h.replay().events.length)
  assert.equal(result.journal.through, h.replay().through)
  assert.equal(result.original.accepted_provider_artifacts, 0)
  assert.equal(result.original.counters.attempt_count, 1)
  assert.equal(result.original.counters.max_attempts, 2)
  assert.equal(h.original.state.snapshot.runs.length, 1)
  assert.ok(h.elapsed < h.c.timeout_ms)
  assert.ok(!JSON.stringify(h.saves).includes(SENTINEL))
})

test('suite retains native forced-commit export failure and performs no review, download, or policy bypass', async () => {
  const h = memoryHarness(undefined, { exportFailure: true })
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io),
    { code: 'native_deliverable_export_rejects_uncommitted_checkpoint_app' })
  assert.equal(h.report.passed, false)
  assert.equal(h.report.status, 'failed')
  assert.equal(h.report.failures.at(-1).code, 'native_deliverable_export_rejects_uncommitted_checkpoint_app')
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
  assert.equal(Object.hasOwn(h.report.intents, 'review'), false)
  assert.equal(h.report.pending_review, null)
  assert.equal(h.reads.download, 0)
  assert.equal(h.downloads.length, 0)
  assert.ok(h.report.original)
  assert.ok(h.report.journal.events.some((entry) =>
    entry.aggregate_id === h.recovered.replacement.id && entry.type === 'run.failed'))
  assert.equal(h.recovered.task.contract.deliverable.form, 'commit_branch')
  assert.equal(h.recovered.task.attempt_count, 1)
  assert.ok(h.report.observations.some((entry) => entry.state.native_failure_payload_sha256.length === 1))
  assert.ok(!JSON.stringify(h.report).includes(SENTINEL))
})

test('suite missing-Artifact negative is explicit rejection, not verified delivery or fabricated provider evidence', async () => {
  const h = memoryHarness(config({ timeoutMs: 15_000, pollMs: 250, settleMs: 2000, missingArtifact: true }))
  await exported('executeSuite')(h.c, h.report, h.io)
  assert.equal(h.report.status, 'expected_missing_artifact_rejection')
  assert.equal(h.report.passed, true)
  assert.equal(h.report.replacement.verified, false)
  assert.equal(h.report.replacement.missing_required_provider_artifact_rejected, true)
  assert.equal(h.report.replacement.provider_artifact_bytes_claimed, false)
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
  assert.equal(h.reads.download, 0)
})

test('--continue cannot create a baseline or begin an unrecorded mutation', async () => {
  for (const existingBaseline of [false, true]) {
    const h = memoryHarness()
    if (existingBaseline) h.report.baseline = driver.captureBaseline(h.snapshot(), h.replay(), h.c)
    await assert.rejects(exported('executeSuite')(h.c, h.report, h.io, { continuation: true }),
      { code: existingBaseline ? 'continue_is_observation_only' : 'new_baseline_or_fresh_item_required' })
    assert.deepEqual(h.mutations, [])
    assert.equal(h.reads.checkInputs, 0)
    assert.deepEqual(h.report.intents, { create: null, recovery: null })
    assert.equal(h.report.passed, false)
  }
})

test('unknown create response is never retried; continuation times out without adopting replacement work', async () => {
  const h = memoryHarness(undefined, { uncertain: 'create_unknown' })
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io),
    { code: 'operation_failed_details_withheld' })
  const firstFailure = clone(h.report.failures[0])
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create'])
  assert.ok(h.report.intents.create)
  assert.ok(Object.values(h.report.identity).every((value) => value === null))
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io, { continuation: true }),
    { code: 'acceptance_deadline_no_replacement' })
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create'])
  assert.equal(h.elapsed, h.c.timeout_ms)
  assert.ok(h.reads.snapshot <= h.c.timeout_ms / h.c.poll_ms + 5)
  assert.deepEqual(h.report.failures[0], firstFailure)
  assert.equal(h.report.failures.length, 2)
  assert.ok(Object.values(h.report.identity).every((value) => value === null))
})

test('lost create response reconciles the exact item/run, but --continue cannot authorize recovery', async () => {
  const h = memoryHarness(undefined, { uncertain: 'create_lost' })
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io))
  assert.equal(h.report.identity.work_item_id, h.original.item.id)
  assert.equal(h.report.identity.mission_id, h.original.mission.id)
  assert.equal(h.report.identity.original_run_id, h.original.run.id)
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io, { continuation: true }),
    { code: 'continue_is_observation_only' })
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create'])
  assert.ok(h.report.original.admission)
  assert.equal(h.report.intents.recovery, null)
  assert.equal(h.report.identity.replacement_run_id, null)
  assert.equal(h.report.failures.length, 2)
})

test('lost recovery response binds the recorded replacement; continuation only pauses for its exact browser gate', async () => {
  const h = memoryHarness(undefined, { uncertain: 'recovery_lost' })
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io))
  const original = clone(h.report.original)
  assert.equal(h.report.identity.replacement_run_id, h.recovered.replacement.id)
  assert.equal(h.report.identity.recovery_id, h.recovered.recovery.id)
  await exported('executeSuite')(h.c, h.report, h.io, { continuation: true })
  assert.equal(h.report.status, 'awaiting_browser_review')
  assert.equal(h.report.passed, false)
  assert.equal(h.report.pending_review.run_id, h.recovered.replacement.id)
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
  assert.equal(Object.hasOwn(h.report.intents, 'review'), false)
  assert.equal(h.reads.context, 1)
  assert.deepEqual(h.report.original, original)
  assert.equal(h.reads.download, 0)
})

test('external browser approval completes exact recorded work without retries or erasing earlier failure history', async () => {
  const h = memoryHarness(undefined, { uncertain: 'recovery_lost' })
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io))
  const failures = clone(h.report.failures)
  const original = clone(h.report.original)
  const intents = clone(h.report.intents)
  await exported('executeSuite')(h.c, h.report, h.io, { continuation: true })
  assert.equal(h.report.status, 'awaiting_browser_review')
  h.setPhase('approved')
  await exported('executeSuite')(h.c, h.report, h.io, { continuation: true })
  assert.equal(h.report.status, 'verified_and_downloaded')
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
  assert.equal(h.reads.checkInputs, 2)
  assert.equal(h.reads.context, 1)
  assert.equal(h.reads.download, 1)
  assert.deepEqual(h.report.intents, intents)
  assert.deepEqual(h.report.original, original)
  assert.deepEqual(h.report.failures, failures)
})

for (const [label, onSnapshot] of [
  ['missing', (state, c) => { state.snapshot.actors = state.snapshot.actors.filter((actor) => actor.id !== c.reviewer) }],
  ['not human', (state, c) => { state.snapshot.actors.find((actor) => actor.id === c.reviewer).kind = 'agent' }],
  ['unauthorized role', (state, c) => { state.snapshot.actors.find((actor) => actor.id === c.reviewer).role = 'viewer' }],
  ['guest', (state, c) => { state.snapshot.actors.find((actor) => actor.id === c.reviewer).role = 'guest' }],
  ['wrong Corp', (state, c) => { state.snapshot.actors.find((actor) => actor.id === c.reviewer).corp_id = id(999) }],
]) {
  test(`suite never mutates when independent reviewer is ${label}`, async () => {
    const c = config({ timeoutMs: 15_000, pollMs: 250, settleMs: 2000 })
    const h = memoryHarness(c, { onSnapshot: (state) => onSnapshot(state, c) })
    await assert.rejects(exported('executeSuite')(h.c, h.report, h.io))
    assert.deepEqual(h.mutations, [])
    assert.equal(h.reads.checkInputs, 0)
  })
}

test('failed intent persistence or changed parent inputs prevents the corresponding native effect', async () => {
  for (const behavior of [{ failIntentSave: true }, { failInputCheck: 1 }, { failInputCheck: 2 }]) {
    const h = memoryHarness(undefined, behavior)
    await assert.rejects(exported('executeSuite')(h.c, h.report, h.io),
      { code: 'operation_failed_details_withheld' })
    assert.deepEqual(h.mutations.map((entry) => entry.operation),
      behavior.failInputCheck === 2 ? ['create'] : [])
    assert.equal(h.report.passed, false)
    assert.ok(!JSON.stringify(h.report).includes(SENTINEL))
  }
})

test('a colliding synthetic item without saved create intent is never adopted or mutated', async () => {
  const h = memoryHarness(undefined, { initial: 'original' })
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io),
    { code: 'unowned_synthetic_issue_collision' })
  assert.deepEqual(h.mutations, [])
  assert.equal(h.report.baseline, null)
})

test('wrong physical base bytes stop before recovery; tampered downloads stop without another attempt', async () => {
  for (const behavior of [{ badBase: true }, { badDownload: true }]) {
    const h = memoryHarness(undefined, behavior)
    if (behavior.badDownload) {
      await exported('executeSuite')(h.c, h.report, h.io)
      assert.equal(h.report.status, 'awaiting_browser_review')
      h.setPhase('approved')
    }
    await assert.rejects(exported('executeSuite')(h.c, h.report, h.io, { continuation: !!behavior.badDownload }), {
      code: behavior.badBase ? 'physical_base_bytes_not_proven' : 'deliverable_download_digest_mismatch',
    })
    assert.deepEqual(h.mutations.map((entry) => entry.operation),
      behavior.badBase ? ['create'] : ['create', 'recovery'])
    assert.equal(h.downloads.length, 0)
    assert.equal(h.report.download, null)
    assert.equal(h.report.passed, false)
  }
})

test('--continue fails closed when recorded work disappears, with no new mission or native retry', async () => {
  const h = memoryHarness()
  await exported('executeSuite')(h.c, h.report, h.io)
  h.setPhase('approved')
  await exported('executeSuite')(h.c, h.report, h.io, { continuation: true })
  const identity = clone(h.report.identity)
  const intents = clone(h.report.intents)
  h.setPhase('empty')
  await assert.rejects(exported('executeSuite')(h.c, h.report, h.io, { continuation: true }),
    { code: 'recorded_factory_item_missing' })
  assert.deepEqual(h.report.identity, identity)
  assert.deepEqual(h.report.intents, intents)
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
})

test('explicit browser-review flag preserves the default two-step flow without a scripted-review mode', async () => {
  const parsed = driver.parseArgs([...requiredPairs().flat(), '--review-via-browser'])
  const c = driver.configuration(receipt(), { ...parsed, timeoutMs: 15_000, settleMs: 2000 }, WIN)
  const h = memoryHarness(c)
  await exported('executeSuite')(c, h.report, h.io)
  assert.equal(h.report.status, 'awaiting_browser_review')
  assert.equal(h.report.passed, false)
  assert.equal(h.reads.download, 0)
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
})

test('browser-review pause cannot be continued with substituted gate identities or reviewer', async () => {
  const h = memoryHarness()
  await exported('executeSuite')(h.c, h.report, h.io)
  for (const field of ['run_id', 'recovery_id', 'reviewer_actor_id']) {
    const report = clone(h.report)
    report.pending_review[field] = id(800)
    await assert.rejects(exported('executeSuite')(h.c, report, h.io, { continuation: true }),
      { code: 'saved_browser_review_identity_changed' })
  }
  assert.deepEqual(h.mutations.map((entry) => entry.operation), ['create', 'recovery'])
})

test('production I/O exposes no review POST and confines injected requests to exact GET routes', async () => {
  const c = config()
  const plan = exported('buildPlan')(c, id(901))
  const requests = []
  const now = () => Date.parse('2026-09-08T16:00:00.000Z')
  const io = exported('createRuntimeIo')(c, plan, {
    now,
    fetchImpl: async (url, settings) => {
      requests.push({ url, settings })
      return new Response(JSON.stringify({ retained: true }), {
        status: 200, headers: { 'content-type': 'application/json' },
      })
    },
    WebSocketImpl: class { constructor() { throw new Error('this test never opens a socket') } },
    execute() { throw new Error('this test never executes a child') },
  })
  for (const name of ['decide', 'review', 'approve', 'post', 'create', 'launch', 'reset', 'enroll']) {
    assert.equal(Object.hasOwn(io, name), false, `${name} must not be a runtime API`)
  }
  await io.context(id(100))
  await io.download({ artifact_id: id(131) })
  assert.deepEqual(requests.map(({ url }) => url), [
    `${EXPECTED.server}/api/corps/${c.corp_id}/factory/work-items/${id(100)}/verification-recoveries?actor_id=${c.actor_id}`,
    `${EXPECTED.server}/api/corps/${c.corp_id}/artifacts/${id(131)}?actor_id=${c.actor_id}`,
  ])
  for (const { settings } of requests) {
    assert.equal(settings.method, 'GET')
    assert.equal(settings.redirect, 'error')
    assert.equal(settings.body, undefined)
    assert.equal(Object.hasOwn(settings.headers, 'authorization'), false)
  }
  assert.throws(() => io.context('unknown/work-item'))
  assert.throws(() => io.download({ artifact_id: 'unknown/artifact' }))
  assert.equal(requests.length, 2)
  assert.throws(() => exported('createRuntimeIo')({ ...c, server_url: 'http://127.0.0.1:18575' }, plan),
    { code: 'runtime_origin_not_authorized' })
})

test('runtime factory adapter never retries an ambiguous injected CLI call', async () => {
  const c = config()
  const plan = exported('buildPlan')(c, id(901))
  let calls = 0
  const io = exported('createRuntimeIo')(c, plan, {
    now: () => Date.parse('2026-09-08T16:00:00.000Z'),
    fetchImpl: async () => { throw new Error('this test never fetches') },
    WebSocketImpl: class { constructor() { throw new Error('this test never opens a socket') } },
    execute(_file, _args, _settings, done) {
      calls += 1
      done(new Error(SENTINEL), SENTINEL, SENTINEL)
    },
  })
  await assert.rejects(io.factory(false, 1000), { code: 'factory_cli_failed_or_uncertain_do_not_retry' })
  await assert.rejects(async () => io.factory(false, 1000), { code: 'native_cli_not_retried' })
  assert.equal(calls, 1)
  await assert.rejects(io.factory(true, 1000), { code: 'factory_cli_failed_or_uncertain_do_not_retry' })
  await assert.rejects(async () => io.factory(true, 1000), { code: 'native_cli_not_retried' })
  assert.equal(calls, 2)
})

// Focused rolling-upgrade regressions: all receipts, archives and runtime I/O
// below remain in memory; no process, file, service or browser is operated.
async function serverUpgradeFixture() {
  const old = receipt()
  old.processes.server.pid = 36944
  old.processes.runner.pid = 14532
  old.processes.web = { role: 'web', pid: 47032, executable: 'C:\\Program Files\\nodejs\\node.exe',
    workspace: EXPECTED.workspace, started_utc: '2026-09-08T16:00:00Z' }
  const args = options({ continuation: true,
    serverUpgradeReceipt: WIN.join(EXPECTED.runtime, 'server-upgrade.json') })
  const h = memoryHarness(driver.configuration(old, args, WIN))
  await driver.executeSuite(h.c, h.report, h.io)
  h.report.status = 'failed'
  h.report.failures.push({ at: 1, code: 'not_exact_provider_free_workspace_lineage' })
  const next = clone(old)
  const build = { source_base_head: '4992e2cdbb9c3d5ed116ca2cb19174f1d77ec800' }
  next.binaries.server = { path: WIN.join(EXPECTED.runtime, 'bin', 'crony-server-upgraded.exe'),
    sha256: 'a'.repeat(64), ...build }
  next.processes.server = { ...old.processes.server, pid: 36945,
    executable: next.binaries.server.path, started_utc: '2026-09-08T18:00:00Z', ...build }
  const upgrade = { schema_version: 1, owner_task: old.owner_task, old_runtime: old, new_runtime: next,
    phase: 'ready', ready_at: '2026-09-08T18:01:00Z',
    old_server_stopped_verified: true, old_server_pid: 36944, new_server_pid: 36945,
    database_unchanged: true, runner_unchanged: true, new_server_source_base_head: build.source_base_head,
    product_source_sha256: {
      paths: ['crates/crony-store/src/checkpoint_retention.rs', 'Cargo.lock'],
      sha256: ['c'.repeat(64), 'd'.repeat(64)],
    } }
  return { h, args, old, next, upgrade, current: driver.configuration(next, args, WIN) }
}

test('server-upgrade option is explicit, continue-only, and confined to the owned runtime directory', async () => {
  const argv = [...requiredPairs().flat(), '--server-upgrade-receipt', WIN.join(EXPECTED.runtime, 'upgrade.json')]
  assert.throws(() => driver.parseArgs(argv), { code: 'server_upgrade_requires_continue' })
  assert.equal(driver.parseArgs([...argv, '--continue']).continuation, true)
  const f = await serverUpgradeFixture()
  assert.throws(() => driver.validateServerUpgrade(f.upgrade, f.next, f.current,
    { ...f.args, serverUpgradeReceipt: 'C:\\elsewhere\\upgrade.json' }, f.h.report, WIN),
  { code: 'path_outside_owned_root' })
  f.upgrade.database_unchanged = false
  assert.throws(() => driver.validateServerUpgrade(f.upgrade, f.next, f.current, f.args, f.h.report, WIN),
    { code: 'server_upgrade_attestation_required' })
  f.upgrade.database_unchanged = true
  f.upgrade.phase = 'new_server_started'
  assert.throws(() => driver.validateServerUpgrade(f.upgrade, f.next, f.current, f.args, f.h.report, WIN),
    { code: 'server_upgrade_attestation_required' })
})

test('server-only upgrade retains the original binding/failure and separately records server provenance read-only', async () => {
  const f = await serverUpgradeFixture()
  const retained = JSON.stringify(f.h.report)
  const evidence = driver.validateServerUpgrade(f.upgrade, f.next, f.current, f.args, f.h.report, WIN)
  assert.equal(JSON.stringify(f.h.report), retained, 'upgrade validation must not alter the old report')
  assert.equal(evidence.old_binding_sha256, f.h.report.binding_sha256)
  assert.notEqual(evidence.new_binding_sha256, evidence.old_binding_sha256)
  assert.equal(evidence.runtime_launch_source_commit, EXPECTED.code_commit)
  assert.equal(evidence.new_server_source_base_head, f.upgrade.new_server_source_base_head)
  assert.deepEqual(evidence.product_source_sha256, f.upgrade.product_source_sha256)
  assert.ok(!JSON.stringify(evidence).includes(SENTINEL))
  const recorded = { ...evidence, retained_report_path: `${f.current.report_path}.previous-${id(910)}.json`,
    retained_report_sha256: sha(retained) }
  f.h.report.server_upgrade = recorded
  assert.throws(() => driver.validateSavedReport(f.h.report, f.current), { code: 'saved_report_binding_mismatch' })
  const before = clone(f.h.mutations)
  await driver.executeSuite(f.current, f.h.report, f.h.io, { continuation: true, serverUpgrade: recorded })
  assert.deepEqual(f.h.mutations, before)
  assert.equal(f.h.report.binding_sha256, evidence.old_binding_sha256)
  assert.equal(f.h.report.status, 'awaiting_browser_review')
  assert.equal(f.h.report.failures.at(-1).code, 'not_exact_provider_free_workspace_lineage')
  assert.equal(f.h.report.server_upgrade.retained_report_sha256, sha(retained))
  assert.deepEqual(driver.validateServerUpgrade(f.upgrade, f.next, f.current, f.args, f.h.report, WIN), recorded)
})

test('upgrade rejects runner/web/CLI changes, a mismatched old binding, and conflicting server provenance', async () => {
  const f = await serverUpgradeFixture()
  for (const role of ['runner', 'web']) {
    const upgrade = clone(f.upgrade)
    upgrade.new_runtime.processes[role].pid += 1
    assert.throws(() => driver.validateServerUpgrade(upgrade, upgrade.new_runtime, f.current, f.args, f.h.report, WIN),
      { code: 'server_upgrade_changed_nonserver_runtime' })
  }
  assert.throws(() => driver.validateServerUpgrade(f.upgrade, f.next, f.current,
    { ...f.args, cli: WIN.join(EXPECTED.runtime, 'bin', 'different-cli.exe') }, f.h.report, WIN),
  { code: 'saved_report_binding_mismatch' })
  const wrongReport = { ...f.h.report, binding_sha256: 'e'.repeat(64) }
  assert.throws(() => driver.validateServerUpgrade(f.upgrade, f.next, f.current, f.args, wrongReport, WIN),
    { code: 'saved_report_binding_mismatch' })
  const wrongBuild = clone(f.upgrade)
  wrongBuild.new_runtime.processes.server.source_base_head = EXPECTED.code_commit
  assert.throws(() => driver.validateServerUpgrade(wrongBuild, wrongBuild.new_runtime, f.current, f.args, f.h.report, WIN),
    { code: 'new_server_build_base_mismatch' })
})

function appendCheckpointReattestation(f) {
  const commandId = id(930)
  f.append('factory.workspace_checkpoint_requested', {
    source_run_id: f.replacement.id, command_id: commandId, expected_head_commit: f.deliverable.head_commit,
    review_re_attestation: true,
  }, { aggregate_id: f.item.id, aggregate_type: 'factory_work_item',
    actor_id: f.c.actor_id, causation_id: f.replacement.id })
  f.append('run.workspace_preserved', {
    workspace: f.run.workspace_path, workspace_branch: f.run.workspace_branch, workspace_base_ref: 'HEAD',
    workspace_base_commit: EXPECTED.source_commit, workspace_fingerprint: f.proof.workspace_fingerprint,
    head_commit: f.deliverable.head_commit, branch_deleted: false, workspace_quarantined: false,
  })
  f.append('runner.command_acknowledged', {
    command_id: commandId, runner_id: f.c.runner_id, command_kind: 'factory_workspace_checkpoint',
  })
}

test('same-verifier native checkpoint re-attestation accepts a real run_id-only VerificationRequest', () => {
  const f = recoveryFixture()
  assert.equal(Object.hasOwn(f.state.snapshot.verification_requests[0], 'id'), false)
  appendCheckpointReattestation(f)
  const result = driver.assessRecovery(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity)
  assert.equal(result.checkpoint_re_attestation.run_id, f.replacement.id)
  assert.equal(result.checkpoint_re_attestation.command_id, id(930))
  assert.equal(result.new_provider_sessions, 0)
  assert.equal(f.state.snapshot.runs.length, 2)
  assert.equal(result.verified, false)
})

test('checkpoint re-attestation cannot target another run, duplicate an ACK, or permit provider usage', () => {
  const f = recoveryFixture()
  appendCheckpointReattestation(f)
  const request = f.replay.events.find((entry) => entry.type === 'factory.workspace_checkpoint_requested')
  request.payload.source_run_id = f.run.id
  assert.throws(() => driver.assessRecovery(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity),
    { code: 'checkpoint_reattest_authority_mismatch' })
  request.payload.source_run_id = f.replacement.id
  f.append('runner.command_acknowledged', {
    command_id: id(931), runner_id: f.c.runner_id, command_kind: 'factory_workspace_checkpoint',
  })
  assert.throws(() => driver.assessRecovery(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity),
    { code: 'checkpoint_reattest_not_exactly_once' })
  f.replay.events.pop()
  f.replay.through -= 1
  f.append('run.usage', { input_tokens: 1, output_tokens: 0, cost_microusd: 0 })
  assert.throws(() => driver.assessRecovery(f.state, f.replay, f.context, f.c, f.plan, f.original, f.identity),
    { code: 'replacement_provider_events_or_unsafe_cleanup' })
})
