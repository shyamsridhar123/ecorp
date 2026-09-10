type FactorySourceScope = {
  corp_id: string
  source_project_owner: string
  source_project_number: number
  source_repository_owner: string
  source_repository_name: string
}

type FactorySelectionItem = FactorySourceScope & { id: string }
type SelectableController = FactorySourceScope & {
  id: string
  desired_state: 'running' | 'paused'
  status: string
  active_work_item_id: string | null
  last_heartbeat_at: string
}

const LIVE_STATES = new Set([
  'watching', 'working', 'blocked', 'needs_decision', 'backing_off',
])
const SOURCE_NAMES = [
  'source_project_owner', 'source_repository_owner', 'source_repository_name',
] as const

function githubName(value: string): string {
  return typeof value === 'string' ? value.trim().toLowerCase() : ''
}

function validSource(scope: FactorySourceScope): boolean {
  return Number.isSafeInteger(scope.source_project_number)
    && scope.source_project_number > 0
    && SOURCE_NAMES.every((field) => githubName(scope[field]) !== '')
}

function sameSource(controller: FactorySourceScope, item: FactorySourceScope): boolean {
  return controller.source_project_number === item.source_project_number
    && SOURCE_NAMES.every((field) => githubName(controller[field]) === githubName(item[field]))
}

function relevance(controller: SelectableController, item: FactorySelectionItem | null | undefined): number {
  // A stale offline active-item pointer cannot displace a current live watcher.
  // Among equally live records, prefer this exact item, then running intake/work.
  return (LIVE_STATES.has(controller.status) ? 8 : 0)
    + (item && controller.active_work_item_id === item.id ? 4 : 0)
    + (controller.desired_state === 'running' ? 2 : 0)
    + (controller.status === 'working' ? 1 : 0)
}

function heartbeat(controller: SelectableController): number {
  const parsed = Date.parse(controller.last_heartbeat_at)
  return Number.isFinite(parsed) ? parsed : -Infinity
}

// Presentation only: consume the current authorized snapshot, retain the original
// record (including its ID/version), and never infer authority or create a watcher.
export function selectFactoryController<C extends SelectableController>(
  controllers: readonly C[],
  selectedItem: FactorySelectionItem | null | undefined,
  corpId: string,
  selectedItemId: string | null = selectedItem?.id ?? null,
): C | undefined {
  if (!corpId || (selectedItemId !== null && selectedItem?.id !== selectedItemId)) {
    return undefined
  }
  if (selectedItem && (selectedItem.corp_id !== corpId || !validSource(selectedItem))) {
    return undefined
  }

  let selected: C | undefined
  for (const candidate of controllers) {
    if (candidate.corp_id !== corpId || !validSource(candidate)) continue
    if (selectedItem && !sameSource(candidate, selectedItem)) continue
    if (!selected) {
      selected = candidate
      continue
    }
    const candidateRank = relevance(candidate, selectedItem)
    const selectedRank = relevance(selected, selectedItem)
    if (candidateRank > selectedRank
      || (candidateRank === selectedRank
        && (heartbeat(candidate) > heartbeat(selected)
          || (heartbeat(candidate) === heartbeat(selected) && candidate.id < selected.id)))) {
      selected = candidate
    }
  }
  return selected
}
