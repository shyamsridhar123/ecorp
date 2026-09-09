import { usesDeterministicHarness } from './missionRuntime.ts'

type MissionDraft<Contract, Policy> = {
  title: string
  description: string
  actorId: string
  strategy: string
  adapter: string
  model: string
  selectedModel: { id: string; supported_reasoning_efforts: readonly string[] } | undefined
  reasoningEffort: string
  source: { repository: string; baseRef: string; baseCommit: string; workspaceConnectionId?: string }
  budgetTokens: number
  deliverableForm: string
  commitDeliverable: boolean
  contract: Contract | null
  customVerification: boolean
  verificationPolicy: Policy
}

/** The builder is shared by preview and creation; it does not allocate budgets. */
export function buildMissionRequest<Contract, Policy>(draft: MissionDraft<Contract, Policy>) {
  const deterministic = usesDeterministicHarness(draft.strategy)
  const model = draft.selectedModel?.id === draft.model ? draft.selectedModel : undefined
  return {
    title: draft.title,
    description: draft.description,
    requested_by: draft.actorId,
    preferred_adapter: draft.adapter,
    preferred_model: !deterministic && model ? draft.model : null,
    reasoning_effort: !deterministic && model?.supported_reasoning_efforts.includes(draft.reasoningEffort)
      ? draft.reasoningEffort : null,
    strategy: draft.strategy,
    source: {
      repository: draft.source.repository,
      base_ref: draft.source.baseRef,
      base_commit: draft.source.baseCommit,
    },
    ...(draft.source.workspaceConnectionId ? { workspace_connection_id: draft.source.workspaceConnectionId } : {}),
    budget_tokens: deterministic ? null : draft.budgetTokens,
    deliverable: {
      form: draft.deliverableForm,
      commit_after_verification: draft.commitDeliverable || draft.deliverableForm === 'commit_branch',
      paths: [],
    },
    contract: draft.contract,
    verification_policy: !deterministic && draft.customVerification ? draft.verificationPolicy : null,
  }
}

export type MissionRequestScope = {
  key: string
  corpId: string
  actorId: string
  body: string
  strategy: string
}

/** Scope the exact serialized body used by both POSTs, not a subset of form fields. */
export function missionRequestScope(corpId: string, actorId: string, body: string): MissionRequestScope {
  const request = JSON.parse(body) as { requested_by?: unknown; strategy?: unknown }
  if (!corpId || !actorId || request.requested_by !== actorId || typeof request.strategy !== 'string') {
    throw new Error('Mission request does not match the current Corp/operator scope.')
  }
  return { key: JSON.stringify([corpId, actorId, body]), corpId, actorId, body, strategy: request.strategy }
}

export type MissionPreviewQuote = {
  strategy: string
  budget_tokens: number
  budget_cost_microusd: number
  tasks: {
    key: string
    title: string
    budget_tokens: number
    budget_cost_microusd: number
    depends_on: string[]
    max_attempts: number
  }[]
}

export type MissionPreviewLoad = {
  scopeKey: string
  status: 'pending' | 'ready' | 'error'
  quote: MissionPreviewQuote | null
  error: string | null
}

export function currentMissionPreview(scope: MissionRequestScope | null, load: MissionPreviewLoad | null) {
  return scope && load?.scopeKey === scope.key ? load : null
}

function readMissionPreview(value: unknown, scope: MissionRequestScope): MissionPreviewQuote {
  const quote = value as MissionPreviewQuote | null
  const amount = (number: unknown) => typeof number === 'number' && Number.isSafeInteger(number) && number >= 0
  if (!quote || quote.strategy !== scope.strategy || !amount(quote.budget_tokens) ||
    !amount(quote.budget_cost_microusd) || !Array.isArray(quote.tasks) || quote.tasks.length === 0 || quote.tasks.length > 8 ||
    quote.tasks.some((task) => !task || typeof task.key !== 'string' || !task.key.trim() ||
      typeof task.title !== 'string' || !amount(task.budget_tokens) || !amount(task.budget_cost_microusd) ||
      !Array.isArray(task.depends_on) || task.depends_on.length > 8 || task.depends_on.some((key) => typeof key !== 'string') ||
      !Number.isSafeInteger(task.max_attempts) || task.max_attempts < 1)) {
    throw new Error('The server returned an incomplete or mismatched allocation preview. Exact allocations are unknown.')
  }
  const keys = new Set(quote.tasks.map((task) => task.key))
  if (keys.size !== quote.tasks.length || quote.tasks.some((task) =>
    new Set(task.depends_on).size !== task.depends_on.length ||
    task.depends_on.some((key) => key === task.key || !keys.has(key)))) {
    throw new Error('The server returned invalid task/dependency keys. Exact allocations are unknown.')
  }
  // These values are quoted by the server. Do not infer or recompute any split.
  return quote
}

function previewError(error: unknown): string {
  const status = error && typeof error === 'object' && 'status' in error ? error.status : null
  if (status === 404 || status === 405 || status === 501) {
    return `Allocation preview is unavailable on this server (HTTP ${status}). No exact task allocations are available; Launch still uses the existing server validation.`
  }
  if (error instanceof SyntaxError) {
    return 'Allocation preview returned no readable JSON; the endpoint may be unavailable on this server. Exact task allocations are unknown; Launch still uses the existing server validation.'
  }
  const detail = error instanceof Error ? error.message : 'The preview request failed.'
  return `Allocation preview unavailable: ${detail.slice(0, 320)}`
}

type PreviewApi = (path: string, init: RequestInit) => Promise<unknown>
type Timer = ReturnType<typeof globalThis.setTimeout>
type PreviewTimers = {
  setTimeout: (callback: () => void, delay: number) => Timer
  clearTimeout: (timer: Timer) => void
}
const nativeTimers: PreviewTimers = {
  setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
  clearTimeout: (timer) => globalThis.clearTimeout(timer),
}

/** Read-only planning only: never calls creation, launch, or an approval endpoint. */
export function startMissionPreview(
  scope: MissionRequestScope,
  post: PreviewApi,
  publish: (load: MissionPreviewLoad) => void,
  timers: PreviewTimers = nativeTimers,
) {
  const controller = new AbortController()
  let active = true
  let requestTimeout: Timer | undefined
  publish({ scopeKey: scope.key, status: 'pending', quote: null, error: null })
  const debounce = timers.setTimeout(() => {
    if (!active || controller.signal.aborted) return
    requestTimeout = timers.setTimeout(() => {
      if (!active || controller.signal.aborted) return
      publish({ scopeKey: scope.key, status: 'error', quote: null, error: 'Allocation preview timed out. Exact task allocations are unknown; retry the read-only preview.' })
      controller.abort()
    }, 15_000)
    void post(`/api/corps/${encodeURIComponent(scope.corpId)}/missions/preview`, {
      method: 'POST', body: scope.body, signal: controller.signal,
    }).then((value) => {
      if (!active || controller.signal.aborted) return
      const quote = readMissionPreview(value, scope)
      publish({ scopeKey: scope.key, status: 'ready', quote, error: null })
    }).catch((error: unknown) => {
      if (!active || controller.signal.aborted) return
      publish({ scopeKey: scope.key, status: 'error', quote: null, error: previewError(error) })
    }).finally(() => {
      if (requestTimeout !== undefined) timers.clearTimeout(requestTimeout)
    })
  }, 300)
  return () => {
    active = false
    timers.clearTimeout(debounce)
    if (requestTimeout !== undefined) timers.clearTimeout(requestTimeout)
    controller.abort()
  }
}
