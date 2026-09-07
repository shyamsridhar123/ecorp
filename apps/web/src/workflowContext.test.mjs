import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { isProviderLiveRun, messagesForMission, missionIdForLink, pendingReviewForRun, relatedWorkOptions, reviewBlockedReason, selectMissionEvidenceRun, workLinkLabel, workflowTaskLabel } from './workflowContext.ts'

const missions = [
  { id: 'm1', title: 'Vendor onboarding' },
  { id: 'm2', title: 'Incident response' },
  { id: 'm3', title: 'Access review' },
]
const tasks = [
  { id: 't1', mission_id: 'm1', title: 'Implement vendor workflow' },
  { id: 't2', mission_id: 'm2', title: 'Implement incident workflow' },
  { id: 't3', mission_id: 'm3', title: 'Review access' },
]
const runs = [
  { id: 'r1', task_id: 't1', status: 'running', artifact_id: 'a1' },
  { id: 'r2', task_id: 't2', status: 'completed', artifact_id: 'a2' },
  { id: 'r3', task_id: 't3', status: 'completed', artifact_id: 'a3' },
]
const context = { missions, tasks, runs }

// Regression: #145, found in the September 7 browser audit. The old room
// selector silently excluded all but the first two missions/tasks/runs.
test('related work includes the third mission, task, run and artifact', () => {
  const options = relatedWorkOptions(context)
  for (const value of ['mission:m3', 'task:t3', 'run:r3', 'artifact:a3']) {
    assert.ok(options.some((option) => option.value === value), value)
  }
  assert.equal(options.length, 12)
})

test('work links name the actual work rather than presenting only opaque IDs', () => {
  assert.equal(workLinkLabel({ kind: 'mission', id: 'm1' }, context), 'Mission · Vendor onboarding')
  assert.equal(workLinkLabel({ kind: 'run', id: 'r1' }, context),
    'Run · Implement vendor workflow · running')
  assert.equal(workLinkLabel({ kind: 'artifact', id: 'a1' }, context),
    'Artifact · Implement vendor workflow')
})

test('related options deduplicate repeated snapshot references', () => {
  const options = relatedWorkOptions({ ...context, runs: [...runs, runs[0]] })
  assert.equal(options.length, 12)
})

// Regression: Factory's contextualMessages discarded Bob's unlinked reply
// even though its parent was correctly linked to the selected mission.
test('mission discussion retains nested replies, including out-of-order snapshots', () => {
  const messages = [
    { id: 'nested', link: null, reply_to_id: 'reply' },
    { id: 'reply', link: null, thread_root_id: 'root' },
    { id: 'root', link: { kind: 'mission', id: 'm1' } },
    { id: 'other', link: { kind: 'mission', id: 'm2' } },
    { id: 'general', link: null },
  ]
  assert.deepEqual(messagesForMission(messages, 'm1', tasks, runs).map(({ id }) => id),
    ['nested', 'reply', 'root'])
})

test('task, run and artifact comments inherit the owning mission only', () => {
  const messages = [
    { id: 'task', link: { kind: 'task', id: 't1' } },
    { id: 'run', link: { kind: 'run', id: 'r1' } },
    { id: 'artifact', link: { kind: 'artifact', id: 'a1' } },
    { id: 'wrong-kind', link: { kind: 'mission', id: 't1' } },
    { id: 'other', link: { kind: 'artifact', id: 'a2' } },
  ]
  assert.deepEqual(messagesForMission(messages, 'm1', tasks, runs).map(({ id }) => id),
    ['task', 'run', 'artifact'])
})

test('unlinked cycles and missing ancestors cannot manufacture mission context', () => {
  const messages = [
    { id: 'a', link: null, reply_to_id: 'b' },
    { id: 'b', link: null, reply_to_id: 'a' },
    { id: 'c', link: null, thread_root_id: 'unavailable' },
  ]
  assert.deepEqual(messagesForMission(messages, 'm1', tasks, runs), [])
})

test('context helpers leave authoritative messages and work objects unchanged', () => {
  const messages = Object.freeze([
    Object.freeze({ id: 'root', link: Object.freeze({ kind: 'mission', id: 'm1' }) }),
  ])
  assert.equal(messagesForMission(messages, 'm1', tasks, runs)[0], messages[0])
  assert.equal(workLinkLabel({ kind: 'task', id: 'unavailable-id' }, context),
    'Task · unavaila')
})

test('work selection follows the owning authorized mission and rejects unknown links', () => {
  for (const [kind, id] of [['mission', 'm1'], ['task', 't1'], ['run', 'r1'], ['artifact', 'a1']]) {
    assert.equal(missionIdForLink({ kind, id }, context), 'm1')
  }
  assert.equal(missionIdForLink({ kind: 'artifact', id: 'missing' }, context), null)
  assert.equal(missionIdForLink({ kind: 'mission', id: 't1' }, context), null)
})

test('human review and automated verification are not advertised as live model sessions', () => {
  assert.equal(isProviderLiveRun({ id: 'r1', status: 'waiting_for_approval' },
    [{ run_id: 'r1', status: 'pending' }]), false)
  assert.equal(isProviderLiveRun({ id: 'r1', status: 'verifying' }), false)
  assert.equal(isProviderLiveRun({ id: 'r1', status: 'running', execution_mode: 'verification_only' }), false)
  assert.equal(isProviderLiveRun({ id: 'r1', status: 'running' }), true)
  assert.equal(isProviderLiveRun({ id: 'r1', status: 'waiting_for_approval' }), true)
  assert.equal(isProviderLiveRun({ id: 'r1', status: 'completed' }), false)
})

test('enterprise task labels explain the legacy studio lane keys', () => {
  assert.equal(workflowTaskLabel('gameplay-systems'), 'Systems and implementation')
  assert.equal(workflowTaskLabel('studio-integration'), 'Integrate and deliver')
})

test('review eligibility explains requester exclusion without bypassing the policy', () => {
  const gate = { type: 'independent_review', roles: ['owner', 'member'], exclude_requester: true }
  assert.match(reviewBlockedReason(gate, { id: 'alice', name: 'Alice', role: 'owner' }, 'alice'),
    /Alice requested this mission.*different authorized room member/)
  assert.equal(reviewBlockedReason(gate, { id: 'bob', name: 'Bob', role: 'member' }, 'alice'), null)
  assert.match(reviewBlockedReason(gate, { id: 'eve', name: 'Eve', role: 'guest' }, 'alice'),
    /guest role cannot decide/)
  assert.equal(reviewBlockedReason({ type: 'human_approval', roles: ['owner'] },
    { id: 'alice', name: 'Alice', role: 'owner' }, 'alice'), null)
})

test('a pending evidence review selects its run rather than a newer active worker', () => {
  const entries = [
    { id: 'newer', task_id: 'new-task', status: 'running' },
    { id: 'reviewed', task_id: 'review-task', status: 'waiting_for_approval' },
  ]
  const review = { run_id: 'reviewed', task_id: 'review-task', status: 'pending' }
  assert.equal(selectMissionEvidenceRun(entries, [review]), entries[1])
  assert.equal(pendingReviewForRun(selectMissionEvidenceRun(entries, [review]), [review]), review)
})

test('a tool-approval wait cannot hide a different pending evidence review', () => {
  const entries = [
    { id: 'tool-wait', task_id: 'tool-task', status: 'waiting_for_approval' },
    { id: 'review-wait', task_id: 'review-task', status: 'waiting_for_approval' },
  ]
  const reviews = [{ run_id: 'review-wait', task_id: 'review-task', status: 'pending' }]
  assert.equal(selectMissionEvidenceRun(entries, reviews), entries[1])
  assert.equal(pendingReviewForRun(entries[0], reviews), undefined)
})

test('review selection rejects mismatched tasks, terminal runs and non-pending requests', () => {
  const entry = { id: 'r1', task_id: 't1', status: 'waiting_for_approval' }
  assert.equal(pendingReviewForRun(entry, [{ run_id: 'r1', task_id: 't2', status: 'pending' }]), undefined)
  assert.equal(pendingReviewForRun(entry, [{ run_id: 'r2', task_id: 't1', status: 'pending' }]), undefined)
  assert.equal(pendingReviewForRun(entry, [{ run_id: 'r1', task_id: 't1', status: 'approved' }]), undefined)
  assert.equal(pendingReviewForRun({ ...entry, status: 'completed' },
    [{ run_id: 'r1', task_id: 't1', status: 'pending' }]), undefined)
})

test('explicit evidence selection stays bound to that run and cannot inherit another review', () => {
  const entries = [
    { id: 'pending', task_id: 't1', status: 'waiting_for_approval' },
    { id: 'history', task_id: 't2', status: 'completed' },
  ]
  const reviews = [{ run_id: 'pending', task_id: 't1', status: 'pending' }]
  const selected = selectMissionEvidenceRun(entries, reviews, 'history')
  assert.equal(selected, entries[1])
  assert.equal(pendingReviewForRun(selected, reviews), undefined)
  assert.equal(selectMissionEvidenceRun(entries, reviews, 'another-mission-run'), entries[0])
})

test('unselected evidence follows remaining reviews and retains snapshot order without mutation', () => {
  const entries = Object.freeze([
    Object.freeze({ id: 'first', task_id: 't1', status: 'waiting_for_approval' }),
    Object.freeze({ id: 'second', task_id: 't2', status: 'waiting_for_approval' }),
  ])
  const reviews = [
    { run_id: 'first', task_id: 't1', status: 'approved' },
    { run_id: 'second', task_id: 't2', status: 'pending' },
  ]
  assert.equal(selectMissionEvidenceRun(entries, reviews), entries[1])
  assert.equal(selectMissionEvidenceRun(entries, []), entries[0])
  assert.equal(selectMissionEvidenceRun([], reviews), undefined)
})

test('a decided run remains selected until the operator deliberately chooses the next review', () => {
  const entries = [
    { id: 'decided', task_id: 't1', status: 'completed' },
    { id: 'still-pending', task_id: 't2', status: 'waiting_for_approval' },
  ]
  const reviews = [
    { run_id: 'decided', task_id: 't1', status: 'approved' },
    { run_id: 'still-pending', task_id: 't2', status: 'pending' },
  ]
  assert.equal(selectMissionEvidenceRun(entries, reviews, 'decided'), entries[0])
  assert.equal(pendingReviewForRun(entries[0], reviews), undefined)
  assert.equal(selectMissionEvidenceRun(entries, reviews, 'still-pending'), entries[1])
})

test('mission UI binds evidence and decisions and pins the target before submitting', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
  const card = app.slice(app.indexOf('function MissionCard('), app.indexOf('function EventRow('))
  assert.match(card, /const evidenceRun = selectMissionEvidenceRun\(runs, verificationRequests, selectedEvidenceRunId\)/)
  assert.match(card, /const pendingRequest = pendingReviewForRun\(evidenceRun, verificationRequests\)/)
  assert.match(card, /const pendingRun = pendingRequest \? evidenceRun : undefined/)
  assert.match(card, /setSelectedEvidenceRunId\(pendingRun\.id\)\s+void onVerificationDecision\(pendingRun, approved\)/)
  assert.match(card, /onClick=\{\(\) => decideEvidence\(true\)\}/)
  assert.match(card, /onClick=\{\(\) => decideEvidence\(false\)\}/)
  assert.match(card, /data-evidence-run-id=\{evidenceRun\.id\}/)
  assert.match(card, /data-review-run-id=\{pendingRun\.id\}/)
  assert.match(card, /run\.task_id === evidenceRun\?\.task_id &&/)
  assert.match(card, /const resumedRunId = await onResume\(resumableRun\)/)
  assert.match(card, /if \(resumedRunId\) setSelectedEvidenceRunId\(resumedRunId\)/)
  assert.doesNotMatch(card, /const latestRun = runs\[0\]/)
})
