export type RunnerModel = {
  id: string
  name: string
  policy_state: string | null
  policy_terms: string | null
  supports_vision: boolean
  supports_reasoning_effort: boolean
  max_prompt_tokens: number | null
  max_context_window_tokens: number | null
  supported_reasoning_efforts: string[]
  default_reasoning_effort: string | null
  billing_multiplier: number | null
}

export type RunnerCapability = {
  name: string
  available: boolean
  detail: string | null
  models: RunnerModel[]
  source_repository?: string | null
  source_base_ref?: string | null
  source_base_commit?: string | null
}

export type RunnerNode = {
  id: string
  corp_id: string
  hostname: string
  os: string
  connected: boolean
  status: 'connected' | 'grace' | 'offline'
  last_seen_at: string
  grace_expires_at: string | null
  capabilities: RunnerCapability[]
}

export type RepositoryTarget = {
  key: string
  repository: string
  baseRef: string
  baseCommit: string
  runnerIds: string[]
  runnerLabels: string[]
}

const MISSION_RUNTIMES = [
  'github-copilot', 'codex', 'claude-code', 'opencode', 'fake-process',
]
const DETERMINISTIC_HARNESS_STRATEGIES = [
  'verification-matrix', 'verification-failure', 'human-approval', 'independent-review',
]

export const STUDIO_STRATEGY = 'studio-swarm'
export const STUDIO_STRATEGY_LABEL = 'Studio team · 3 Copilot agents'

export function usesDeterministicHarness(strategy: string): boolean {
  return DETERMINISTIC_HARNESS_STRATEGIES.includes(strategy)
}

export function workspaceCapability(runner: RunnerNode): RunnerCapability | undefined {
  return runner.capabilities.find(
    (capability) =>
      capability.name === 'workspace-isolation' &&
      capability.available &&
      capability.source_repository &&
      capability.source_base_ref &&
      capability.source_base_commit,
  )
}

export function runnerMatchesRepository(
  runner: RunnerNode,
  target: RepositoryTarget,
): boolean {
  const capability = workspaceCapability(runner)
  return Boolean(
    capability?.source_repository?.toLowerCase() === target.repository.toLowerCase() &&
    capability.source_base_ref === target.baseRef &&
    capability.source_base_commit?.toLowerCase() === target.baseCommit.toLowerCase(),
  )
}

/** Runners advertise runtimes; mission workers need not exist before planning. */
export function availableRunnerAdapters(
  data: { runners: readonly RunnerNode[] } | null,
  target?: RepositoryTarget,
): RunnerCapability[] {
  if (!data) return []
  const connectedRunners = data.runners.filter(
    (runner) => runner.connected && (!target || runnerMatchesRepository(runner, target)),
  )
  return Array.from(
    connectedRunners
      .flatMap((runner) => runner.capabilities)
      .filter(
        (capability) => capability.available && MISSION_RUNTIMES.includes(capability.name),
      )
      .reduce((adapters, capability) => {
        const existing = adapters.get(capability.name)
        const modelsById = new Map(
          (existing?.models ?? []).map((model) => [model.id, model]),
        )
        for (const model of capability.models) {
          const prior = modelsById.get(model.id)
          if (!prior || (prior.policy_state === 'disabled' && model.policy_state !== 'disabled')) {
            modelsById.set(model.id, model)
          }
        }
        adapters.set(capability.name, {
          ...capability,
          detail: existing?.detail ?? capability.detail,
          models: Array.from(modelsById.values()),
        })
        return adapters
      }, new Map<string, RunnerCapability>())
      .values(),
  )
}

export function selectMissionAdapter(
  strategy: string,
  adapters: readonly RunnerCapability[],
  requestedAdapter: string,
): RunnerCapability | undefined {
  const required = strategy === STUDIO_STRATEGY
    ? 'github-copilot'
    : usesDeterministicHarness(strategy) ? 'fake-process' : null
  if (required) return adapters.find((adapter) => adapter.name === required && adapter.available)
  if (requestedAdapter) {
    return adapters.find((adapter) => adapter.name === requestedAdapter && adapter.available)
  }
  return MISSION_RUNTIMES
    .map((name) => adapters.find((adapter) => adapter.name === name && adapter.available))
    .find(Boolean)
}

export function missionRuntimeError(
  strategy: string,
  adapter: RunnerCapability | undefined,
  modelId: string,
): string | null {
  if (strategy === STUDIO_STRATEGY && (!adapter?.available || adapter.name !== 'github-copilot')) {
    return 'Studio team requires GitHub Copilot on a connected runner for the selected repository, ref, and commit.'
  }
  if (!adapter?.available) {
    return 'Select a repository with a connected runner advertising the required runtime.'
  }
  if (!usesDeterministicHarness(strategy) && modelId &&
    !adapter.models.some((model) => model.id === modelId && model.policy_state !== 'disabled')) {
    return 'The selected model is unavailable on this source runner. Choose an enabled model or the provider default.'
  }
  return null
}
