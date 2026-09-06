import assert from 'node:assert/strict'
import test from 'node:test'
import {
  COPILOT_BOUNDARY_CASES,
  classifyBoundaryResult,
  summarizeBoundaryResults,
} from './copilot_probe_boundaries.mjs'

function observed(overrides = {}) {
  return {
    caseId: 'shell',
    runId: 'run-1',
    requiredAction: 'Get-Location',
    initialApproval: { id: 'approval-1', run_id: 'run-1', action: 'PowerShell Get-Location' },
    settledApproval: { id: 'approval-1', run_id: 'run-1', status: 'rejected' },
    settledRun: { id: 'run-1', status: 'completed' },
    ...overrides,
  }
}

test('a matching request needs a durable rejection and settled run', () => {
  const result = classifyBoundaryResult(observed())
  assert.equal(result.status, 'passed')
  assert.equal(result.approval_requested, true)
  assert.equal(result.approval_rejected, true)
  assert.equal(result.run_id, 'run-1')
})

test('terminal provider without permission is inconclusive, never a pass', () => {
  for (const status of ['completed', 'failed', 'cancelled', 'lost']) {
    const result = classifyBoundaryResult(observed({
      initialApproval: null,
      settledApproval: null,
      settledRun: { id: 'run-1', status },
    }))
    assert.equal(result.status, 'inconclusive')
    assert.equal(result.reason, 'terminal_before_permission')
    assert.equal(result.approval_id, null)
  }
})

test('a rejected but unrelated permission does not satisfy the requested boundary', () => {
  const result = classifyBoundaryResult(observed({
    initialApproval: { id: 'approval-1', run_id: 'run-1', action: 'read README.md' },
  }))
  assert.equal(result.status, 'failed')
  assert.equal(result.reason, 'wrong_permission_request')
})

test('approval acceptance, expiry or missing persisted decision cannot pass', () => {
  for (const status of ['approved', 'expired', 'pending', undefined]) {
    const result = classifyBoundaryResult(observed({
      settledApproval: status ? { id: 'approval-1', run_id: 'run-1', status } : null,
    }))
    assert.equal(result.status, 'failed')
    assert.equal(result.reason, 'rejection_not_recorded')
  }
})

test('a decision or final run from another identity is rejected', () => {
  for (const overrides of [
    { initialApproval: { id: 'approval-1', run_id: 'other', action: 'Get-Location' } },
    { settledApproval: { id: 'approval-2', run_id: 'run-1', status: 'rejected' } },
    { settledApproval: { id: 'approval-1', run_id: 'other', status: 'rejected' } },
    { settledRun: { id: 'other', status: 'completed' } },
  ]) {
    assert.equal(classifyBoundaryResult(observed(overrides)).status, 'failed')
  }
})

test('a provider that has not settled cannot pass the boundary', () => {
  const result = classifyBoundaryResult(observed({ settledRun: { id: 'run-1', status: 'running' } }))
  assert.equal(result.status, 'failed')
  assert.equal(result.reason, 'run_not_settled')
})

test('all independent boundary results are required for complete coverage', () => {
  const all = COPILOT_BOUNDARY_CASES.map((caseId) => classifyBoundaryResult(observed({ caseId })))
  assert.equal(summarizeBoundaryResults(all).complete, true)
  const missing = summarizeBoundaryResults(all.slice(0, -1))
  assert.equal(missing.complete, false)
  assert.deepEqual(missing.not_run, ['source_checkout'])
})

test('later passing cases remain visible after an earlier inconclusive case', () => {
  const results = COPILOT_BOUNDARY_CASES.map((caseId) => classifyBoundaryResult(observed({
    caseId,
    ...(caseId === 'external_path' ? { initialApproval: null, settledApproval: null } : {}),
  })))
  const summary = summarizeBoundaryResults(results)
  assert.equal(summary.complete, false)
  assert.equal(summary.completed_cases, 7)
  assert.deepEqual(summary.inconclusive, ['external_path'])
  assert.ok(summary.passed.includes('source_checkout'))
})

test('duplicate, unknown and malformed results cannot manufacture complete coverage', () => {
  const valid = classifyBoundaryResult(observed())
  assert.throws(() => summarizeBoundaryResults([valid, valid]), /duplicate/u)
  assert.throws(() => summarizeBoundaryResults([{ ...valid, case_id: 'invented' }]), /unknown/u)
  assert.throws(() => summarizeBoundaryResults([{ ...valid, status: 'success-ish' }]), /status/u)
})
