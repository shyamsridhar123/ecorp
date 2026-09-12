export type FactoryRecoveryCommandMode =
  | 'verifier-only'
  | 'source-correction'
  | 'checkpoint-verification'

type NativeRecoveryContext = {
  work_item: { state: string }
  task_id: string
  source_run_id: string
  workspace_fingerprint: string | null
  expected_head_commit: string | null
  checkpoint_verification?: boolean
  checkpoint_cancellation_event_id?: string | null
  recoveries: readonly { status: string }[]
}

const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu
function uuid(value: unknown): value is string {
  return typeof value === 'string' && UUID.test(value)
    && value.replaceAll('-', '') !== '0'.repeat(32)
}

export function needsFactoryRecoveryContext(state: string | undefined): boolean {
  // This only requests current server context; no mission status grants recovery.
  return ['verification_failed', 'cancelled', 'running', 'blocked', 'awaiting_approval']
    .includes(state ?? '')
}

export function factoryRecoveryModes(context: NativeRecoveryContext | null): FactoryRecoveryCommandMode[] {
  if (!context) return []
  if (context.checkpoint_cancellation_event_id != null
    && (context.checkpoint_verification !== true || context.work_item.state !== 'cancelled')) return []
  if (context.checkpoint_verification === true) {
    const complete = uuid(context.task_id) && uuid(context.source_run_id)
      && typeof context.workspace_fingerprint === 'string'
      && /^[0-9a-f]{64}$/iu.test(context.workspace_fingerprint)
      && typeof context.expected_head_commit === 'string'
      && /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/iu.test(context.expected_head_commit)
    const reconciliation = context.work_item.state !== 'cancelled'
      || uuid(context.checkpoint_cancellation_event_id)
    const active = context.recoveries.some((entry) => ['authorized', 'running'].includes(entry.status))
    return complete && reconciliation && !active
      && ['cancelled', 'running', 'verification_failed', 'blocked', 'awaiting_approval'].includes(context.work_item.state)
      ? ['checkpoint-verification'] : []
  }
  // In particular, a preserved hash or a cancelled mission is not an exception
  // to cancelled Factory intent. Only the server's optional marker permits repair.
  return context.work_item.state === 'verification_failed'
    ? ['verifier-only', 'source-correction'] : []
}

export function factoryRecoveryConnection(policy: Record<string, unknown>): string | null | undefined {
  const value = policy.workspace_connection_id
  return value == null ? null : uuid(value) ? value : undefined
}

export function factoryRecoveryBlocksProviderResume(
  contextExpected: boolean,
  context: NativeRecoveryContext | null,
): boolean {
  return contextExpected && (!context || context.checkpoint_verification === true
    || context.work_item.state === 'cancelled')
}
