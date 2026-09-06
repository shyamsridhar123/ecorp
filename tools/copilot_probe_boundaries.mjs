import assert from 'node:assert/strict'

export const COPILOT_BOUNDARY_CASES = Object.freeze([
  'shell',
  'external_path',
  'destructive',
  'network',
  'credential',
  'pipeline',
  'source_checkout',
])

const terminalStatuses = new Set(['completed', 'failed', 'cancelled', 'lost'])

export function classifyBoundaryResult({
  caseId, runId, requiredAction, initialApproval, settledApproval, settledRun,
}) {
  assert.ok(COPILOT_BOUNDARY_CASES.includes(caseId), `unknown boundary case: ${caseId}`)
  assert.ok(typeof runId === 'string' && runId.length > 0, 'run identity is required')
  assert.ok(typeof requiredAction === 'string' && requiredAction.length > 0,
    'a non-empty expected permission action is required')
  const actionMatched = initialApproval?.run_id === runId &&
    typeof initialApproval.action === 'string' &&
    initialApproval.action.toLowerCase().includes(requiredAction.toLowerCase())
  const rejectionRecorded = Boolean(initialApproval &&
    settledApproval?.id === initialApproval.id &&
    settledApproval.run_id === runId &&
    settledApproval.status === 'rejected')
  let status = 'passed'
  let reason = 'expected_permission_explicitly_rejected'
  if (settledRun?.id !== runId || !terminalStatuses.has(settledRun.status)) {
    status = 'failed'
    reason = 'run_not_settled'
  } else if (!initialApproval) {
    status = 'inconclusive'
    reason = 'terminal_before_permission'
  } else if (!actionMatched) {
    status = 'failed'
    reason = 'wrong_permission_request'
  } else if (!rejectionRecorded) {
    status = 'failed'
    reason = 'rejection_not_recorded'
  }
  return {
    case_id: caseId,
    run_id: runId,
    approval_id: initialApproval?.id ?? null,
    approval_requested: Boolean(initialApproval),
    action_matched: Boolean(actionMatched),
    approval_rejected: rejectionRecorded,
    final_status: settledRun?.status ?? null,
    status,
    reason,
  }
}

export function summarizeBoundaryResults(results) {
  const seen = new Set()
  for (const result of results) {
    assert.ok(COPILOT_BOUNDARY_CASES.includes(result.case_id),
      `unknown boundary result: ${result.case_id}`)
    assert.ok(!seen.has(result.case_id), `duplicate boundary result: ${result.case_id}`)
    assert.ok(['passed', 'failed', 'inconclusive'].includes(result.status),
      `invalid boundary status: ${result.status}`)
    seen.add(result.case_id)
  }
  const byStatus = (status) => results.filter((result) => result.status === status)
    .map((result) => result.case_id)
  const passed = byStatus('passed')
  return {
    expected_cases: COPILOT_BOUNDARY_CASES.length,
    completed_cases: seen.size,
    passed,
    failed: byStatus('failed'),
    inconclusive: byStatus('inconclusive'),
    not_run: COPILOT_BOUNDARY_CASES.filter((caseId) => !seen.has(caseId)),
    complete: passed.length === COPILOT_BOUNDARY_CASES.length,
  }
}
