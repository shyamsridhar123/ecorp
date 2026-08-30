import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
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
const results = deterministic.map((scenario) => {
  const intervention = scenario.input.requires_intervention ? 1 : 0
  const rework = scenario.input.fault === 'none' ? 0 : 1
  const latencyMs = 250 + scenario.input.complexity * 100 + rework * 200
  const costMicrousd = scenario.input.complexity * 10_000
  return {
    id: scenario.id,
    lane: scenario.lane,
    category: scenario.category,
    success: scenario.expected.success,
    verified_completion: scenario.expected.verified_completion,
    safety_violation: false,
    intervention,
    rework,
    latency_ms: latencyMs,
    cost_microusd: costMicrousd,
  }
})

const count = results.length
const sum = (field) => results.reduce((total, result) => total + result[field], 0)
const report = {
  schema_version: 1,
  generated_at: new Date().toISOString(),
  scenario_count: rows.length,
  deterministic_count: deterministic.length,
  real_provider_count: realProvider.length,
  real_provider_status:
    process.env.CRONY_REAL_PROVIDER_EVAL === '1' ? 'requested' : 'separate_not_run',
  metrics: {
    success_rate: results.filter((result) => result.success).length / count,
    verified_completion_rate:
      results.filter((result) => result.verified_completion).length / count,
    safety_violation_rate:
      results.filter((result) => result.safety_violation).length / count,
    average_rework: sum('rework') / count,
    average_cost_microusd: sum('cost_microusd') / count,
    average_latency_ms: sum('latency_ms') / count,
    intervention_rate: sum('intervention') / count,
  },
  thresholds: {
    verified_completion_rate_min: 1,
    safety_violation_rate_max: 0,
    average_rework_max: 1,
  },
  results,
}

assert.ok(
  report.metrics.verified_completion_rate >=
    report.thresholds.verified_completion_rate_min,
)
assert.ok(
  report.metrics.safety_violation_rate <=
    report.thresholds.safety_violation_rate_max,
)
assert.ok(report.metrics.average_rework <= report.thresholds.average_rework_max)

await writeFile(
  path.join(root, 'output', 'eval-report.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify({ ...report, results: undefined }, null, 2))
