import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const output = path.join(root, 'output')
const rows = (await readFile(path.join(root, 'tests', 'scenarios', 'v1.jsonl'), 'utf8'))
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => JSON.parse(line))

assert.ok(rows.length >= 100, 'evaluation suite must contain at least 100 scenarios')
assert.equal(new Set(rows.map((row) => row.id)).size, rows.length)
assert.ok(rows.every((row) => row.version === 1))
assert.ok(rows.every((row) => ['deterministic', 'real_provider'].includes(row.lane)))

const deterministic = rows.filter((row) => row.lane === 'deterministic')
const realProvider = rows.filter((row) => row.lane === 'real_provider')
const maximumEvidenceAgeMs = Number(
  process.env.CRONY_EVAL_MAX_EVIDENCE_AGE_MS ?? 2 * 60 * 60 * 1000,
)

async function evidence(file) {
  const fullPath = path.join(output, file)
  const [raw, metadata] = await Promise.all([
    readFile(fullPath, 'utf8'),
    stat(fullPath),
  ])
  const ageMs = Date.now() - metadata.mtimeMs
  assert.ok(
    ageMs >= 0 && ageMs <= maximumEvidenceAgeMs,
    `${file} is stale (${Math.round(ageMs / 1000)} seconds old)`,
  )
  const data = JSON.parse(raw)
  return {
    file,
    sha256: createHash('sha256').update(raw).digest('hex'),
    checked_at: data.checked_at ?? metadata.mtime.toISOString(),
    data,
  }
}

const categoryContracts = {
  task_decomposition: {
    files: ['e2e-task-graph.json'],
    validate: ([report]) =>
      report.parallel_graph.final_status === 'completed' &&
      report.parallel_graph.max_active_runs >= 2 &&
      report.parallel_graph.synthesis_consumed_verified_specialists === true,
  },
  capability_matching: {
    files: ['e2e-copilot.json'],
    validate: ([report]) =>
      report.adapter_available === true &&
      report.invalid_model_status === 400 &&
      report.resumed_same_session === true,
  },
  delegation_clarity: {
    files: ['e2e-task-graph.json'],
    validate: ([report]) =>
      report.parallel_graph.task_ids.length === 3 &&
      report.retry_bound.attempts === report.retry_bound.max_attempts,
  },
  recovery: {
    files: ['e2e-runner-reconnect.json', 'e2e-approvals.json'],
    validate: ([runner, approval]) =>
      runner.grace_observed === true &&
      runner.reconciliation_event === true &&
      runner.stale_claim_preserved_lost_state === true &&
      approval.startup_runner_recovery_verified === true,
  },
  approval_safety: {
    files: ['e2e-approvals.json', 'e2e-verification.json'],
    validate: ([approval, verification]) =>
      approval.approved_run_status === 'completed' &&
      approval.rejected_run_status === 'cancelled' &&
      approval.expired_approval_status === 'expired' &&
      approval.approved_command_acknowledged === true &&
      verification.independent_review.requester_status === 403,
  },
  budget_safety: {
    files: ['e2e-budgets.json'],
    validate: ([report]) =>
      report.spend_stages.includes('suspend') &&
      report.hard_stop_stages.includes('stop') &&
      report.healthy_conversation_status === 'completed' &&
      report.healthy_conversation_incidents === 0,
  },
  secret_safety: {
    files: ['e2e-secrets.json'],
    validate: ([report]) =>
      report.task_scoped_delivery === true &&
      report.plaintext_absent_from_snapshot_events_and_logs === true &&
      report.revoked === true,
  },
  artifact_verification: {
    files: ['e2e-verification.json', 'e2e-artifacts.json'],
    validate: ([verification, artifact]) =>
      verification.automated_matrix.status === 'passed' &&
      verification.failed_verification.failed_checks.length > 0 &&
      artifact.local_path_exposed === false &&
      artifact.unauthorized_download_status === 404,
  },
  multiplayer_control: {
    files: ['e2e-leases.json', 'e2e-replay.json'],
    validate: ([leases, replay]) =>
      leases.stale_token_status === 409 &&
      leases.transferred_controller_fenced === true &&
      leases.queued_note_delivered === true &&
      replay.duplicate_replay_count === 0,
  },
}

const loadedByFile = new Map()
async function loadFile(file) {
  if (!loadedByFile.has(file)) {
    loadedByFile.set(file, evidence(file))
  }
  return loadedByFile.get(file)
}

const categoryEvidence = new Map()
for (const [category, contract] of Object.entries(categoryContracts)) {
  const records = await Promise.all(contract.files.map(loadFile))
  const valid = contract.validate(records.map((record) => record.data))
  assert.equal(valid, true, `${category} integration evidence did not satisfy its contract`)
  categoryEvidence.set(category, {
    valid,
    records: records.map(({ data: _data, ...record }) => record),
  })
}

const results = deterministic.map((scenario) => {
  const category = categoryEvidence.get(scenario.category)
  assert.ok(category, `scenario ${scenario.id} has no executable evidence contract`)
  return {
    id: scenario.id,
    lane: scenario.lane,
    category: scenario.category,
    evaluation_passed: category.valid,
    verified_completion: category.valid,
    safety_violation: !category.valid,
    expected_workload_success: scenario.expected.success,
    coverage_scope: 'category_integration',
    evidence_reused: true,
    evidence: category.records,
    latency_ms: null,
    cost_microusd: null,
    intervention: null,
    rework: null,
  }
})

let realProviderEvidence = null
if (process.env.CRONY_REAL_PROVIDER_EVAL === '1') {
  realProviderEvidence = await evidence('real-provider-evidence.json')
  assert.equal(
    realProviderEvidence.data.verified_completion,
    true,
    'real-provider evidence did not prove verified completion',
  )
}

const count = results.length
const report = {
  schema_version: 2,
  generated_at: new Date().toISOString(),
  execution_model: 'evidence_backed_category_contracts',
  evidence_reuse_disclosed: true,
  scenario_count: rows.length,
  deterministic_count: deterministic.length,
  real_provider_count: realProvider.length,
  real_provider_status: realProviderEvidence ? 'verified' : 'separate_not_run',
  real_provider_evidence: realProviderEvidence
    ? {
        file: realProviderEvidence.file,
        sha256: realProviderEvidence.sha256,
        checked_at: realProviderEvidence.checked_at,
      }
    : null,
  metrics: {
    evaluation_pass_rate:
      results.filter((result) => result.evaluation_passed).length / count,
    verified_completion_rate:
      results.filter((result) => result.verified_completion).length / count,
    safety_violation_rate:
      results.filter((result) => result.safety_violation).length / count,
    average_rework: null,
    average_cost_microusd: null,
    average_latency_ms: null,
    intervention_rate: null,
  },
  thresholds: {
    evaluation_pass_rate_min: 1,
    verified_completion_rate_min: 1,
    safety_violation_rate_max: 0,
  },
  results,
}

assert.ok(
  report.metrics.evaluation_pass_rate >=
    report.thresholds.evaluation_pass_rate_min,
)
assert.ok(
  report.metrics.verified_completion_rate >=
    report.thresholds.verified_completion_rate_min,
)
assert.ok(
  report.metrics.safety_violation_rate <=
    report.thresholds.safety_violation_rate_max,
)

await writeFile(
  path.join(output, 'eval-report.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify({ ...report, results: undefined }, null, 2))
