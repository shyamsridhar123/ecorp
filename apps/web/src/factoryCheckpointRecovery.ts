export type FactoryRecoveryCommandMode =
  | 'verifier-only'
  | 'source-correction'
  | 'checkpoint-verification'

type NativeRecoveryContext = {
  work_item: { id: string; corp_id: string; mission_id: string | null; version: number; state: string }
  mission_id: string
  task_id: string
  source_run_id: string
  workspace_fingerprint: string | null
  expected_head_commit: string | null
  checkpoint_verification?: boolean
  checkpoint_verification_available?: boolean
  checkpoint_source_correction?: boolean
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
  if (context.checkpoint_source_correction === true && context.checkpoint_verification !== true) return []
  if (context.checkpoint_cancellation_event_id != null
    && (context.checkpoint_verification !== true || context.work_item.state !== 'cancelled')) return []
  if (context.checkpoint_verification === true) {
    const hasHead = typeof context.expected_head_commit === 'string'
      && /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/iu.test(context.expected_head_commit)
    // A failed provider correction may have no exported head. Only the server's
    // explicit source-correction permission admits that null; missing/malformed heads
    // and legacy checkpoint contexts still fail closed.
    const correctionWithoutExportedHead = context.expected_head_commit === null
      && context.checkpoint_verification_available === false
      && context.checkpoint_source_correction === true
    const complete = uuid(context.task_id) && uuid(context.source_run_id)
      && typeof context.workspace_fingerprint === 'string'
      && /^[0-9a-f]{64}$/iu.test(context.workspace_fingerprint)
      && (hasHead || correctionWithoutExportedHead)
    const reconciliation = context.work_item.state !== 'cancelled'
      || uuid(context.checkpoint_cancellation_event_id)
    const active = context.recoveries.some((entry) => ['authorized', 'running'].includes(entry.status))
    if (!complete || !reconciliation || active
      || !['cancelled', 'running', 'verification_failed', 'blocked', 'awaiting_approval'].includes(context.work_item.state)) return []
    // The server separately proves provider correction authority. A checkpoint
    // flag or cancellation repair alone never grants it, including on old servers.
    const modes: FactoryRecoveryCommandMode[] = []
    // Missing availability retains the legacy checkpoint behavior. An explicit
    // denial never falls through to ordinary verifier/provider recovery.
    if (hasHead && context.checkpoint_verification_available !== false) modes.push('checkpoint-verification')
    if (context.checkpoint_source_correction === true && context.work_item.state !== 'cancelled') {
      modes.push('source-correction')
    }
    return modes
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
    || context.checkpoint_source_correction === true
    || context.work_item.state === 'cancelled')
}

export function factoryContractRevisionSource(
  context: NativeRecoveryContext | null,
  scope: { corpId: string; missionId: string; itemId: string; version: number },
  taskId: string,
): string | null {
  if (!context || context.work_item.id !== scope.itemId
    || context.work_item.corp_id !== scope.corpId || context.work_item.version !== scope.version
    || context.work_item.mission_id !== scope.missionId || context.mission_id !== scope.missionId
    || context.task_id !== taskId || !uuid(context.task_id) || !uuid(context.source_run_id)
    || context.work_item.state === 'cancelled'
    || context.recoveries.some((entry) => ['authorized', 'running'].includes(entry.status))) return null
  if ((context.checkpoint_verification === true || context.checkpoint_source_correction === true)
    && !factoryRecoveryModes(context).includes('source-correction')) return null
  // The exact endpoint selects this identity, including a provider-free verifier.
  // Never substitute its older provider ancestor from the visible run list.
  return context.source_run_id
}
