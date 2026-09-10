import type { RepositoryTarget, RunnerCapability, RunnerModel, RunnerNode } from './missionRuntime'

export type CodingAgent = 'github-copilot' | 'codex' | 'claude-code'
export type ConnectionStatus =
  | 'connecting' | 'ready' | 'needs_sign_in' | 'not_installed'
  | 'offline' | 'incompatible' | 'failed'
export type SetupStatus = 'queued' | 'running' | 'needs_sign_in' | 'succeeded' | 'failed' | 'cancelled'
export type SourceIdentity = {
  repository: string
  repository_id?: string | null
  base_ref: string
  base_commit: string
}
export type WorkspaceConnection = {
  id: string
  corp_id: string
  room_id: string
  created_by: string
  runner_id: string
  label: string
  agent: CodingAgent
  source: SourceIdentity | null
  status: ConnectionStatus
  detail: string
  models: RunnerModel[]
  version: number
  last_checked_at: string | null
  runner_connected: boolean
  created_at: string
  updated_at: string
}
export type GitHubRepositoryChoice = {
  id: string
  repository: string
  default_branch: string
  private: boolean
  can_push: boolean
}
export type NativeSignIn = {
  provider: string
  verification_uri: string
  user_code: string | null
  expires_at: string
  input_kind?: 'authorization_code' | null
  input_id?: string | null
}
export type SetupReport = {
  status: SetupStatus
  detail: string
  connection_status: ConnectionStatus | null
  source: SourceIdentity | null
  models: RunnerModel[]
  account_login: string | null
  sign_in: NativeSignIn | null
  repositories: GitHubRepositoryChoice[]
}
export type SetupOperation = {
  id: string
  corp_id: string
  room_id: string
  actor_id: string
  runner_id: string
  connection_id: string | null
  kind: string
  status: SetupStatus
  report: SetupReport | null
  created_at: string
  updated_at: string
  expires_at: string
}
export type WorkspaceConnections = {
  connections: WorkspaceConnection[]
  operations: SetupOperation[]
  selected_connection_id: string | null
}
export type SetupResponse = {
  connection: WorkspaceConnection | null
  operation: SetupOperation
  replayed: boolean
}
export type ConnectionRepository =
  | { kind: 'github'; repository: string; repository_id?: string; base_ref: string; account: 'machine' | 'personal' }
  | { kind: 'local'; directory: string; base_ref: string }
  | { kind: 'advertised'; source: SourceIdentity }

export const CODING_AGENTS: ReadonlyArray<{ id: CodingAgent; label: string }> = [
  { id: 'github-copilot', label: 'GitHub Copilot' },
  { id: 'codex', label: 'Codex' },
  { id: 'claude-code', label: 'Claude Code' },
]

export function connectionLabel(agent: string): string {
  return CODING_AGENTS.find((candidate) => candidate.id === agent)?.label ?? agent
}

export function connectionScope(corp: string, room: string, actor: string): string {
  return JSON.stringify([corp, room, actor])
}

export function pendingSetup(operation: SetupOperation): boolean {
  return ['queued', 'running', 'needs_sign_in'].includes(operation.status)
}

/** Stable lifecycle invalidation key; excludes heartbeat and capability churn. */
export function connectionRunnerRevision(runners: readonly RunnerNode[]): string {
  const records = runners.map((runner) =>
    JSON.stringify([runner.id, runner.corp_id, runner.connected, runner.status]))
  return JSON.stringify(records.sort())
}

/** Revalidation hint only: endpoint readiness remains authoritative and untouched. */
export function connectionsNeedPresenceRefresh(
  connections: readonly WorkspaceConnection[],
  runners: readonly RunnerNode[],
): boolean {
  return connections.some((connection) => {
    const runner = runners.find((candidate) =>
      candidate.id === connection.runner_id && candidate.corp_id === connection.corp_id)
    return connection.runner_connected !== (runner?.connected === true)
  })
}

export function connectionTarget(
  connection: WorkspaceConnection,
  runners: readonly RunnerNode[],
): RepositoryTarget | undefined {
  if (!connection.source) return undefined
  const runner = runners.find((candidate) => candidate.id === connection.runner_id)
  return {
    key: `connection:${connection.id}`,
    repository: connection.source.repository,
    baseRef: connection.source.base_ref,
    baseCommit: connection.source.base_commit,
    runnerIds: [connection.runner_id],
    runnerLabels: [runner ? `${runner.hostname} · ${runner.os}` : 'Saved machine'],
    workspaceConnectionId: connection.id,
  }
}

export function connectionRuntime(
  connection: WorkspaceConnection,
  runners: readonly RunnerNode[],
): RunnerCapability | undefined {
  if (connection.status !== 'ready' || !connection.source) return undefined
  const identity = connection.source
  const runner = runners.find((candidate) =>
    candidate.id === connection.runner_id && candidate.connected)
  const source = runner?.capabilities.find((capability) =>
    capability.workspace_connection_id === connection.id &&
    capability.name === 'workspace-isolation' && capability.available &&
    capability.source_repository === identity.repository &&
    capability.source_base_ref === identity.base_ref &&
    capability.source_base_commit === identity.base_commit)
  if (!source) return undefined
  return runner?.capabilities.find((capability) =>
    capability.workspace_connection_id === connection.id &&
    capability.name === connection.agent && capability.available)
}

export function connectionStatusLabel(connection: WorkspaceConnection): string {
  if (!connection.runner_connected &&
    ['ready', 'connecting', 'offline'].includes(connection.status)) return 'Machine offline'
  return {
    connecting: 'Checking connection',
    ready: 'Ready',
    needs_sign_in: 'Sign-in needed',
    not_installed: 'Agent not installed',
    offline: 'Machine offline',
    incompatible: 'Setup needs attention',
    failed: 'Check failed',
  }[connection.status]
}

/** A persisted choice is never replaced merely because its machine disconnected. */
export function chooseSavedConnection(
  connections: readonly WorkspaceConnection[],
  selectedId: string | null,
): WorkspaceConnection | undefined {
  return selectedId ? connections.find((connection) => connection.id === selectedId) : undefined
}

export function signInUrl(instruction: NativeSignIn): string | undefined {
  try {
    const url = new URL(instruction.verification_uri)
    const providerHosts: Record<string, readonly string[]> = {
      github: ['github.com'],
      'github-copilot': ['github.com'],
      codex: ['auth.openai.com'],
      'claude-code': ['claude.ai', 'console.anthropic.com', 'platform.claude.com'],
    }
    const hosts = providerHosts[instruction.provider]
    if (url.protocol !== 'https:' || url.username || url.password ||
      (url.port && url.port !== '443') || !hosts?.includes(url.hostname)) return undefined
    return url.href
  } catch {
    return undefined
  }
}
