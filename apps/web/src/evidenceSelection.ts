type EvidenceSelectionScope = {
  server: string
  corpId: string
  actorId: string
  missionId: string
}

type EvidenceSelectionStorage = Pick<Storage, 'getItem' | 'setItem'>
const RUN_ID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u

// Presentation state only. A remembered ID never grants visibility or authority;
// the caller must still resolve it in the current authorized mission snapshot.
export function evidenceSelectionKey(scope: EvidenceSelectionScope): string {
  return `ecorp:evidence-run:v1:${JSON.stringify([
    scope.server, scope.corpId, scope.actorId, scope.missionId,
  ])}`
}

export function readEvidenceSelection(
  storage: () => EvidenceSelectionStorage,
  key: string,
): string | null {
  try {
    const value = storage().getItem(key)
    // null means first visit. An invalid/unreadable saved context instead needs
    // an explicit selection; it must not fall through to another pending run.
    return value === null ? null : RUN_ID.test(value) ? value : ''
  } catch {
    return ''
  }
}

export function rememberEvidenceSelection(
  storage: () => EvidenceSelectionStorage,
  key: string,
  runId: string,
): boolean {
  if (!RUN_ID.test(runId)) return false
  try {
    storage().setItem(key, runId)
    return true
  } catch {
    return false
  }
}
