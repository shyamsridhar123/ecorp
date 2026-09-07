export type WorkLink = {
  kind: 'mission' | 'task' | 'run' | 'artifact'
  id: string
}

type WorkMission = { id: string; title: string }
type WorkTask = { id: string; mission_id: string; title: string }
type WorkRun = {
  id: string
  task_id: string
  status: string
  artifact_id?: string | null
}
type WorkMessage = {
  id: string
  link: WorkLink | null
  reply_to_id?: string | null
  thread_root_id?: string | null
}

export type WorkContext = {
  missions: readonly WorkMission[]
  tasks: readonly WorkTask[]
  runs: readonly WorkRun[]
}

export function isProviderLiveRun(
  run: { id: string; status: string; execution_mode?: string | null },
  reviews: readonly { run_id: string; status: string }[] = [],
): boolean {
  if (run.execution_mode && run.execution_mode !== 'provider') return false
  if (reviews.some((review) => review.run_id === run.id && review.status === 'pending')) return false
  return ['provisioning', 'starting', 'running', 'waiting_for_input', 'waiting_for_approval']
    .includes(run.status)
}

export function workflowTaskLabel(key: string): string {
  const labels: Record<string, string> = {
    'visual-direction': 'Product experience',
    'gameplay-systems': 'Systems and implementation',
    'quality-verification': 'Quality and verification',
    'studio-integration': 'Integrate and deliver',
    deliver: 'Deliver the outcome',
  }
  return labels[key] ?? key.replaceAll('-', ' ')
}

export function reviewBlockedReason(
  gate: { type: string; roles: readonly string[]; exclude_requester?: boolean },
  actor: { id: string; name?: string; role: string },
  requesterId: string,
): string | null {
  if (!gate.roles.includes(actor.role)) {
    return `Your ${actor.role} role cannot decide this review. An authorized room member with one of these roles must review: ${gate.roles.join(', ')}.`
  }
  if (gate.type === 'independent_review' && gate.exclude_requester && actor.id === requesterId) {
    return `${actor.name ?? 'You'} requested this mission. Its independent-review rule requires a different authorized room member to accept or reject the evidence.`
  }
  return null
}

type ReviewRun = { id: string; task_id: string; status: string }
type RunReview = { run_id: string; task_id: string; status: string }

export function pendingReviewForRun<T extends RunReview>(
  run: ReviewRun | undefined,
  reviews: readonly T[],
): T | undefined {
  if (!run || run.status !== 'waiting_for_approval') return undefined
  return reviews.find((review) =>
    review.status === 'pending' && review.run_id === run.id && review.task_id === run.task_id,
  )
}

// Evidence, downloads and review decisions must use the same exact run. A tool
// approval on a newer run must not conceal another worker's evidence review.
export function selectMissionEvidenceRun<T extends ReviewRun>(
  runs: readonly T[],
  reviews: readonly RunReview[],
  selectedRunId: string | null = null,
): T | undefined {
  if (selectedRunId !== null) return runs.find((run) => run.id === selectedRunId)
  return runs.find((run) => pendingReviewForRun(run, reviews)) ?? runs[0]
}

export function missionIdForLink(link: WorkLink, context: WorkContext): string | null {
  if (link.kind === 'mission') {
    return context.missions.some((mission) => mission.id === link.id) ? link.id : null
  }
  const taskId = link.kind === 'task'
    ? link.id
    : context.runs.find((run) =>
        link.kind === 'run' ? run.id === link.id : run.artifact_id === link.id,
      )?.task_id
  const missionId = context.tasks.find((task) => task.id === taskId)?.mission_id
  return missionId && context.missions.some((mission) => mission.id === missionId)
    ? missionId
    : null
}

export function workLinkLabel(link: WorkLink, context: WorkContext): string {
  if (link.kind === 'mission') {
    return `Mission · ${context.missions.find((mission) => mission.id === link.id)?.title ?? link.id.slice(0, 8)}`
  }
  if (link.kind === 'task') {
    return `Task · ${context.tasks.find((task) => task.id === link.id)?.title ?? link.id.slice(0, 8)}`
  }
  const run = context.runs.find((candidate) =>
    link.kind === 'run' ? candidate.id === link.id : candidate.artifact_id === link.id,
  )
  const task = context.tasks.find((candidate) => candidate.id === run?.task_id)
  const label = link.kind === 'run' ? 'Run' : 'Artifact'
  return `${label} · ${task?.title ?? link.id.slice(0, 8)}${link.kind === 'run' && run ? ` · ${run.status}` : ''}`
}

export function relatedWorkOptions(context: WorkContext) {
  const links: WorkLink[] = [
    ...context.missions.map(({ id }) => ({ kind: 'mission' as const, id })),
    ...context.tasks.map(({ id }) => ({ kind: 'task' as const, id })),
    ...context.runs.flatMap((run): WorkLink[] => [
      { kind: 'run', id: run.id },
      ...(run.artifact_id ? [{ kind: 'artifact' as const, id: run.artifact_id }] : []),
    ]),
  ]
  return Array.from(new Map(links.map((link) => [
    `${link.kind}:${link.id}`,
    { value: `${link.kind}:${link.id}`, label: workLinkLabel(link, context) },
  ])).values())
}

// Derive discussion from already-authorized snapshot objects. A reply inherits its
// thread's context; this never creates tasks, grants authority, or fetches hidden work.
export function messagesForMission<T extends WorkMessage>(
  messages: readonly T[],
  missionId: string,
  tasks: readonly WorkTask[],
  runs: readonly WorkRun[],
): T[] {
  const taskIds = new Set(tasks.filter((task) => task.mission_id === missionId).map(({ id }) => id))
  const missionRuns = runs.filter((run) => taskIds.has(run.task_id))
  const linked = new Set([
    `mission:${missionId}`,
    ...Array.from(taskIds, (id) => `task:${id}`),
    ...missionRuns.flatMap((run) => [
      `run:${run.id}`,
      ...(run.artifact_id ? [`artifact:${run.artifact_id}`] : []),
    ]),
  ])
  const included = new Set(messages
    .filter((message) => message.link && linked.has(`${message.link.kind}:${message.link.id}`))
    .map(({ id }) => id))
  let changed = true
  while (changed) {
    changed = false
    for (const message of messages) {
      if (included.has(message.id)) continue
      if (
        (message.reply_to_id && included.has(message.reply_to_id)) ||
        (message.thread_root_id && included.has(message.thread_root_id))
      ) {
        included.add(message.id)
        changed = true
      }
    }
  }
  return messages.filter((message) => included.has(message.id))
}
