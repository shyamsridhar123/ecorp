import { missionOriginScope } from './missionOriginContext.ts'
import type { MissionOriginApi, MissionOriginScope } from './missionOriginContext.ts'

export type MissionResultScope = MissionOriginScope & Readonly<{
  workItemId: string
  sourceRepository: string
}>

export type MissionResultPublication = Readonly<{
  id: string
  state: 'requested' | 'publishing' | 'branch_pushed' | 'pull_request_created' | 'published'
  version: number
  task_id: string
  run_id: string
  source_deliverable_id: string
  artifact_id: string
  target_repository: string
  base_ref: string
  branch: string
  commit_sha: string
  pull_request_number: number | null
  pull_request_url: string | null
  pull_request_state: 'open' | 'closed' | 'merged' | null
  pull_request_draft: boolean | null
  failed: boolean
}>

// Compatible with the existing source-download callback. No workspace path,
// provider output, authorization snapshot or credential is retained.
export type MissionResultDeliverable = Readonly<{
  id: string
  task_id: string
  run_id: string
  artifact_id: string
  form: 'commit_branch'
  file_name: string
  uri: string
  sha256: string
  media_type: string
  bytes: number
  provenance_signature: string
  verification_sha256: string
  base_commit: string
  head_commit: string
  branch: string
  integration_state: 'not_applicable' | 'ready_for_review' | 'published' | 'integrated'
  retention_until: string
}>

export type MissionResultContext = Readonly<{
  corp_id: string
  mission_id: string
  work_item_id: string
  work_item_version: number
  source_repository: string
  publication: MissionResultPublication | null
  deliverable: MissionResultDeliverable | null
}>

export type MissionResultLoad =
  | { scope: MissionResultScope; status: 'pending' | 'unavailable'; context: null }
  | { scope: MissionResultScope; status: 'ready'; context: MissionResultContext }

export type MissionResultRun = Readonly<{
  id: string
  task_id?: string
  verification_sha256?: string | null
  deliverable_sha256?: string | null
}>

export type MissionResultPresentation = {
  state: 'loading' | 'unavailable' | 'none' | 'mismatch' | 'pending' | 'failed' | 'pr_created' | 'published'
  publication: MissionResultPublication | null
  deliverable: MissionResultDeliverable | null
  pullRequestUrl: string | null
  resultRunId: string | null
}

function identifier(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 128 &&
    !/[^!-~]/.test(value) && value !== '.' && value !== '..'
}

function text(value: unknown, max = 500): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= max &&
    value.trim() === value && [...value].every((character) => {
      const code = character.charCodeAt(0)
      return code > 0x1f && (code < 0x7f || code > 0x9f)
    })
}

function repository(value: unknown): value is string {
  return text(value, 240) &&
    /^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\/[A-Za-z0-9_.-]+$/.test(value) &&
    !value.endsWith('/.') && !value.endsWith('/..')
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function positiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
}

function hash(value: unknown, commit = false): string | null {
  return typeof value === 'string' && (value.length === 64 || (commit && value.length === 40)) &&
    !/[^a-fA-F0-9]/.test(value) ? value.toLowerCase() : null
}

function sameRepository(left: unknown, right: string): boolean {
  return repository(left) && left.toLowerCase() === right.toLowerCase()
}

function githubUrl(value: unknown, source: string, kind: 'issues' | 'pull', number: unknown): string | null {
  // Native publication accepts trailing PR slash runs. Match that suffix
  // directly without normalizing the URL or changing the issue-link contract.
  const pattern = kind === 'pull'
    ? /^https:\/\/github\.com\/([A-Za-z0-9-]+\/[A-Za-z0-9_.-]+)\/(pull)\/([1-9][0-9]*)\/*$/
    : /^https:\/\/github\.com\/([A-Za-z0-9-]+\/[A-Za-z0-9_.-]+)\/(issues|pull)\/([1-9][0-9]*)$/
  const match = typeof value === 'string' ? pattern.exec(value) : null
  return match && match[0] === value && sameRepository(match[1], source) &&
    match[2] === kind && positiveInteger(number) && match[3] === String(number) ? match[0] : null
}

function containsIdentity(value: unknown, id: string): boolean {
  return Array.isArray(value) && value.every(identifier) &&
    new Set(value).size === value.length && value.includes(id)
}

/** Recreate on view/identity/API/role changes or an explicit retry, including A -> B -> A. */
export function missionResultScope(input: {
  corpId: string | null | undefined
  actorId: string | null | undefined
  missionId: string | null | undefined
  roomId: string | null | undefined
  workItemId: string | null | undefined
  sourceRepository: string | null | undefined
}): MissionResultScope {
  const origin = missionOriginScope(input.corpId, input.actorId, input.missionId, input.roomId)
  if (![origin.corpId, origin.actorId, origin.missionId, origin.roomId, input.workItemId].every(identifier) ||
    !identifier(input.workItemId) || !repository(input.sourceRepository)) {
    throw new Error('Result context requires an explicit operator, mission, room, work item and repository.')
  }
  return Object.freeze({
    ...origin, workItemId: input.workItemId, sourceRepository: input.sourceRepository,
    key: JSON.stringify([origin.key, input.workItemId, input.sourceRepository]),
  })
}

export function currentMissionResult(scope: MissionResultScope | null, load: MissionResultLoad | null) {
  // Scope object identity is the view-generation fence, not merely equal ID strings.
  return scope && load?.scope === scope ? load : null
}

function readContext(value: unknown, scope: MissionResultScope): MissionResultContext {
  if (!record(value) || !record(value.work_item) || !Array.isArray(value.source_deliverables)) {
    throw new Error('Invalid result context')
  }
  const item = value.work_item
  if (item.id !== scope.workItemId || item.corp_id !== scope.corpId || item.mission_id !== scope.missionId ||
    !positiveInteger(item.version) || typeof item.source_repository_owner !== 'string' ||
    typeof item.source_repository_name !== 'string' ||
    !sameRepository(`${item.source_repository_owner}/${item.source_repository_name}`, scope.sourceRepository) ||
    !githubUrl(item.source_issue_url, scope.sourceRepository, 'issues', item.source_issue_number)) {
    throw new Error('Mismatched result context')
  }
  const identity = {
    corp_id: scope.corpId, mission_id: scope.missionId, work_item_id: scope.workItemId,
    work_item_version: item.version, source_repository: scope.sourceRepository,
  }
  // This endpoint does not echo actor/room. Those remain bound to this request
  // generation; current Operate and mission-room authorization belong to the server.
  if (value.publication === null) return { ...identity, publication: null, deliverable: null }
  const p = value.publication
  if (!record(p) || p.corp_id !== scope.corpId || p.mission_id !== scope.missionId ||
    p.factory_work_item_id !== scope.workItemId ||
    !identifier(p.id) || !identifier(p.run_id) || !identifier(p.task_id) ||
    !identifier(p.source_deliverable_id) || !identifier(p.artifact_id) ||
    !positiveInteger(p.version) || !sameRepository(p.target_repository, scope.sourceRepository) ||
    p.source_issue_number !== item.source_issue_number ||
    !githubUrl(p.source_issue_url, scope.sourceRepository, 'issues', item.source_issue_number) ||
    !text(p.base_ref) || !text(p.branch) ||
    typeof p.state !== 'string' ||
    !['requested', 'publishing', 'branch_pushed', 'pull_request_created', 'published'].includes(p.state) ||
    !(p.failure_detail === null || typeof p.failure_detail === 'string') || !record(p.provenance)) {
    throw new Error('Invalid publication')
  }
  const matches = value.source_deliverables.filter((entry) =>
    record(entry) && entry.id === p.source_deliverable_id)
  if (matches.length !== 1) throw new Error('Missing or ambiguous published deliverable')
  const d = matches[0] as Record<string, unknown>
  const proof = p.provenance
  const dp = proof.deliverable
  const target = proof.target
  const sourceIssue = proof.source_issue
  const commit = hash(p.commit_sha, true)
  const bytesDigest = hash(d.sha256)
  const verificationDigest = hash(d.verification_sha256)
  const baseCommit = hash(d.base_commit, true)
  const signature = hash(d.provenance_signature)
  const uri = `/api/corps/${encodeURIComponent(scope.corpId)}/artifacts/${encodeURIComponent(p.artifact_id)}`
  if (d.corp_id !== scope.corpId || d.run_id !== p.run_id || d.task_id !== p.task_id ||
    d.artifact_id !== p.artifact_id || d.form !== 'commit_branch' || d.uri !== uri ||
    !commit || !bytesDigest || !verificationDigest || !baseCommit || !signature ||
    hash(d.head_commit, true) !== commit || !text(d.branch) ||
    !text(d.file_name, 255) || /[\\/:]/.test(d.file_name) || ['.', '..'].includes(d.file_name) ||
    !text(d.media_type, 128) || !/^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/i.test(d.media_type) ||
    !positiveInteger(d.bytes) ||
    typeof d.integration_state !== 'string' ||
    !['not_applicable', 'ready_for_review', 'published', 'integrated'].includes(d.integration_state) ||
    !text(d.retention_until, 64) || !Number.isFinite(Date.parse(d.retention_until)) ||
    ![1, 2, 3].includes(Number(proof.schema_version)) || typeof proof.schema_version !== 'number' ||
    proof.factory_work_item_id !== scope.workItemId || proof.mission_id !== scope.missionId ||
    (sourceIssue !== undefined && (!record(sourceIssue) || sourceIssue.number !== item.source_issue_number ||
      !githubUrl(sourceIssue.url, scope.sourceRepository, 'issues', item.source_issue_number))) ||
    !containsIdentity(proof.task_ids, p.task_id) || !containsIdentity(proof.run_ids, p.run_id) ||
    hash(proof.verification_sha256) !== verificationDigest ||
    !record(dp) || dp.id !== d.id || dp.artifact_id !== d.artifact_id ||
    hash(dp.sha256) !== bytesDigest || hash(dp.head_commit, true) !== commit ||
    hash(dp.base_commit, true) !== baseCommit || dp.source_branch !== d.branch ||
    !record(target) || !sameRepository(target.repository, scope.sourceRepository) ||
    target.base_ref !== p.base_ref || target.branch !== p.branch || hash(target.commit, true) !== commit) {
    throw new Error('Inconsistent publication evidence')
  }
  const hasPr = p.state === 'pull_request_created' || p.state === 'published'
  let prUrl: string | null = null
  let prNumber: number | null = null
  let prState: MissionResultPublication['pull_request_state'] = null
  let prDraft: boolean | null = null
  if (hasPr) {
    const url = githubUrl(p.pull_request_url, scope.sourceRepository, 'pull', p.pull_request_number)
    const pr = proof.pull_request
    const state = typeof p.pull_request_state === 'string' ? p.pull_request_state.toLowerCase() : ''
    if (!url || !positiveInteger(p.pull_request_number) ||
      !['open', 'closed', 'merged'].includes(state) || typeof p.pull_request_draft !== 'boolean' ||
      hash(p.pull_request_head_sha, true) !== commit || p.pull_request_is_cross_repository !== false ||
      typeof p.pull_request_head_repository_owner !== 'string' ||
      p.pull_request_head_repository_owner.toLowerCase() !== scope.sourceRepository.split('/')[0].toLowerCase() ||
      !text(p.pull_request_base_ref) || !record(pr) ||
      pr.url !== p.pull_request_url || pr.number !== p.pull_request_number ||
      pr.state !== p.pull_request_state || pr.draft !== p.pull_request_draft ||
      hash(pr.head_sha, true) !== commit || pr.base_ref !== p.pull_request_base_ref ||
      pr.head_repository_owner !== p.pull_request_head_repository_owner || pr.is_cross_repository !== false) {
      throw new Error('Inconsistent pull request')
    }
    prUrl = url
    prNumber = p.pull_request_number
    prState = state as NonNullable<MissionResultPublication['pull_request_state']>
    prDraft = p.pull_request_draft
  } else if (p.pull_request_url !== null || p.pull_request_number !== null || proof.pull_request !== null) {
    throw new Error('Premature pull request')
  }
  // Binding/format checks are not signature verification, renewed publication
  // authority, remote PR liveness, application hosting or provider completion.
  return {
    ...identity,
    publication: {
      id: p.id, state: p.state as MissionResultPublication['state'], version: p.version,
      task_id: p.task_id, run_id: p.run_id, source_deliverable_id: p.source_deliverable_id,
      artifact_id: p.artifact_id, target_repository: p.target_repository as string,
      base_ref: p.base_ref, branch: p.branch, commit_sha: commit,
      pull_request_number: prNumber, pull_request_url: prUrl,
      pull_request_state: prState, pull_request_draft: prDraft,
      failed: typeof p.failure_detail === 'string' && p.failure_detail.trim().length > 0,
    },
    deliverable: {
      id: p.source_deliverable_id, task_id: p.task_id, run_id: p.run_id, artifact_id: p.artifact_id,
      form: 'commit_branch', file_name: d.file_name, uri, sha256: bytesDigest,
      media_type: d.media_type, bytes: d.bytes, provenance_signature: signature,
      verification_sha256: verificationDigest, base_commit: baseCommit, head_commit: commit,
      branch: d.branch, integration_state: d.integration_state as MissionResultDeliverable['integration_state'],
      retention_until: d.retention_until,
    },
  }
}

type Timer = ReturnType<typeof globalThis.setTimeout>
type ResultTimers = {
  setTimeout: (callback: () => void, delay: number) => Timer
  clearTimeout: (timer: Timer) => void
}
const nativeTimers: ResultTimers = {
  setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
  clearTimeout: (timer) => globalThis.clearTimeout(timer),
}

/** One read only: no retries, credentials, provider access or publication mutation. */
export function startMissionResultRead(
  scope: MissionResultScope,
  get: MissionOriginApi,
  publish: (load: MissionResultLoad) => void,
  timers: ResultTimers = nativeTimers,
) {
  const controller = new AbortController()
  let active = true
  const finish = (load: MissionResultLoad) => {
    if (!active) return
    active = false
    timers.clearTimeout(timeout)
    publish(load)
  }
  publish({ scope, status: 'pending', context: null })
  const timeout = timers.setTimeout(() => {
    if (!active) return
    finish({ scope, status: 'unavailable', context: null })
    controller.abort()
  }, 15_000)
  void Promise.resolve().then(() => {
    if (!active) return undefined
    return get(
      `/api/corps/${encodeURIComponent(scope.corpId)}/factory/work-items/${encodeURIComponent(scope.workItemId)}/publication-context?actor_id=${encodeURIComponent(scope.actorId)}`,
      { method: 'GET', cache: 'no-store', signal: controller.signal },
    )
  }).then((value) => {
    if (!active) return
    finish({ scope, status: 'ready', context: readContext(value, scope) })
  }).catch(() => {
    // Missing endpoints, denied access and malformed data never become absence.
    // In particular, do not retain or render raw server/transport errors.
    finish({ scope, status: 'unavailable', context: null })
  })
  return () => {
    active = false
    timers.clearTimeout(timeout)
    controller.abort()
  }
}

/** Omit selectedRun for a mission-level result; supply even just its ID for a pinned historical view. */
export function missionResultPresentation(
  scope: MissionResultScope | null,
  load: MissionResultLoad | null,
  selectedRun?: MissionResultRun | null,
): MissionResultPresentation {
  const empty = (state: MissionResultPresentation['state'], resultRunId: string | null = null) => ({
    state, publication: null, deliverable: null, pullRequestUrl: null, resultRunId,
  })
  if (!scope) return empty('unavailable')
  if (!load) return empty('loading')
  const current = currentMissionResult(scope, load)
  if (!current) return empty('unavailable')
  if (current.status !== 'ready') return empty(current.status === 'pending' ? 'loading' : 'unavailable')
  const { publication, deliverable } = current.context
  if (!publication) return empty('none')
  if (!deliverable) return empty('unavailable')
  if (selectedRun !== undefined && selectedRun !== null) {
    if (!record(selectedRun) || !identifier(selectedRun.id)) return empty('unavailable')
    if (selectedRun.id !== publication.run_id) return empty('mismatch', publication.run_id)
    // A conflicting tuple for the same run is inconsistent data, not an older
    // review selection. Do not offer navigation as a way around that conflict.
    if ((selectedRun.task_id !== undefined && selectedRun.task_id !== publication.task_id) ||
      (selectedRun.verification_sha256 !== undefined && hash(selectedRun.verification_sha256) !== deliverable.verification_sha256) ||
      (selectedRun.deliverable_sha256 !== undefined && hash(selectedRun.deliverable_sha256) !== deliverable.sha256)) {
      return empty('unavailable')
    }
  }
  const state = publication.failed ? 'failed'
    : publication.state === 'published' ? 'published'
    : publication.state === 'pull_request_created' ? 'pr_created' : 'pending'
  return { state, publication, deliverable, pullRequestUrl: publication.pull_request_url, resultRunId: publication.run_id }
}
