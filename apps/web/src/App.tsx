import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import './App.css'

type Actor = {
  id: string
  name: string
  kind: 'human' | 'agent' | 'service'
  role: string
}

type Agent = {
  id: string
  actor_id: string
  name: string
  role: string
  adapter: string
  status: 'idle' | 'starting' | 'working' | 'blocked' | 'reviewing' | 'offline'
  station: string | null
  current_run_id: string | null
  accent: string
}

type Mission = {
  id: string
  title: string
  status: 'draft' | 'ready' | 'running' | 'completed' | 'failed' | 'cancelled'
  created_at: string
}

type Task = {
  id: string
  mission_id: string
  title: string
  objective: string
  status: string
  assigned_agent_id: string | null
}

type Run = {
  id: string
  task_id: string
  agent_id: string
  runner_id: string
  status: string
  summary: string | null
  artifact_path: string | null
  artifact_sha256: string | null
}

type Lease = {
  agent_id: string
  actor_id: string
  expires_at: string
}

type QueuedMessage = {
  id: string
  agent_id: string
  actor_id: string
  text: string
  status: string
  created_at: string
}

type DomainEvent = {
  seq: number
  id: string
  type: string
  actor_id: string | null
  aggregate_type: string
  aggregate_id: string
  payload: Record<string, unknown>
  created_at: string
}

type EntityLink = {
  kind: 'mission' | 'task' | 'run' | 'artifact'
  id: string
}

type RoomMessage = {
  id: string
  room_id: string
  actor_id: string
  thread_root_id: string | null
  reply_to_id: string | null
  body: string
  mentions: string[]
  link: EntityLink | null
  created_at: string
}

type BrowserSocketMessage =
  | { type: 'ready'; corp_id: string; replayed_through: number }
  | { type: 'event'; event: DomainEvent }

type SnapshotResponse = {
  snapshot: {
    corp: { id: string; name: string }
    actors: Actor[]
    rooms: { id: string; name: string; purpose: string }[]
    agents: Agent[]
    missions: Mission[]
    tasks: Task[]
    runs: Run[]
    room_messages: RoomMessage[]
    leases: Lease[]
    queued_messages: QueuedMessage[]
    events: DomainEvent[]
  }
  runners: {
    id: string
    hostname: string
    os: string
    connected: boolean
    status: 'connected' | 'grace' | 'offline'
    last_seen_at: string
    grace_expires_at: string | null
  }[]
}

type BootstrapResponse = {
  corp_id: string
  room_id: string
  alice_actor_id: string
  bob_actor_id: string
  eve_actor_id: string
  manager_agent_id: string
  worker_agent_id: string
}

const API_URL = import.meta.env.VITE_CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const DEFAULT_MISSION = 'Prepare a verified launch-readiness brief for the Crony Corp alpha.'

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_URL}${path}`, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...init?.headers,
    },
  })
  const body = await response.json()
  if (!response.ok) {
    throw new Error(body.error ?? `${response.status} ${response.statusText}`)
  }
  return body as T
}

function shortId(value: string | null | undefined): string {
  return value ? value.slice(0, 8) : '—'
}

function time(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(value))
}

function leaseTokenKey(actorId: string, agentId: string): string {
  return `${actorId}:${agentId}`
}

function StatusMark({ status }: { status: Agent['status'] }) {
  return <span className={`status-mark status-${status}`} aria-label={status} />
}

function AgentAvatar({ agent }: { agent: Agent }) {
  return (
    <div className={`agent-avatar accent-${agent.accent} agent-${agent.status}`} aria-hidden="true">
      <span className="avatar-head" />
      <span className="avatar-body" />
      <span className="avatar-shadow" />
    </div>
  )
}

function AgentDesk({
  agent,
  lease,
  leaseToken,
  actor,
  otherHuman,
  queuedCount,
  onClaim,
  onRelease,
  onTransfer,
  onEmergencyStop,
  onMessage,
}: {
  agent: Agent
  lease: Lease | undefined
  leaseToken: string | undefined
  actor: Actor
  otherHuman: Actor | undefined
  queuedCount: number
  onClaim: (agent: Agent) => Promise<void>
  onRelease: (agent: Agent, token: string) => Promise<void>
  onTransfer: (agent: Agent, token: string, toActor: Actor) => Promise<void>
  onEmergencyStop: (agent: Agent) => Promise<void>
  onMessage: (agent: Agent, text: string, token: string | undefined) => Promise<void>
}) {
  const [text, setText] = useState('')
  const ownsLease = lease?.actor_id === actor.id
  const holderLabel = lease ? (ownsLease ? 'You hold control' : 'Controlled by another operator') : 'Unclaimed'

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!text.trim()) return
    await onMessage(agent, text, leaseToken)
    setText('')
  }

  return (
    <article className={`agent-desk desk-${agent.status}`} data-testid={`agent-${agent.name}`}>
      <div className="desk-room-label">{agent.role}</div>
      <div className="desk-stage">
        <div className="work-station">
          <span className="monitor" />
          <span className="desk-surface" />
        </div>
        <AgentAvatar agent={agent} />
        {agent.status !== 'idle' ? (
          <div className="activity-bubble">{agent.station ?? agent.status}</div>
        ) : null}
      </div>
      <div className="desk-card">
        <div>
          <div className="agent-name">
            <StatusMark status={agent.status} />
            {agent.name}
          </div>
          <div className="agent-meta">
            {agent.adapter} · run {shortId(agent.current_run_id)}
          </div>
        </div>
        <div className={`lease-label ${ownsLease ? 'lease-owned' : ''}`}>{holderLabel}</div>
      </div>
      <div className="desk-actions">
        <button type="button" className="button button-secondary" onClick={() => onClaim(agent)}>
          {ownsLease && leaseToken ? 'Renew control' : 'Claim control'}
        </button>
        {ownsLease && leaseToken ? (
          <button
            type="button"
            className="button button-quiet"
            onClick={() => onRelease(agent, leaseToken)}
          >
            Release
          </button>
        ) : null}
        <span className="queue-count">{queuedCount} queued</span>
      </div>
      {ownsLease && leaseToken && otherHuman ? (
        <button
          type="button"
          className="transfer-control"
          onClick={() => onTransfer(agent, leaseToken, otherHuman)}
        >
          Transfer control to {otherHuman.name}
        </button>
      ) : null}
      {agent.current_run_id && ['owner', 'admin', 'manager'].includes(actor.role) ? (
        <button
          type="button"
          className="emergency-stop"
          onClick={() => onEmergencyStop(agent)}
        >
          Emergency stop
        </button>
      ) : null}
      <form className="agent-message" onSubmit={submit}>
        <input
          aria-label={`Message ${agent.name}`}
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={ownsLease ? 'Send live direction…' : 'Queue a note…'}
        />
        <button className="button button-ink" type="submit">
          Send
        </button>
      </form>
    </article>
  )
}

function MissionCard({
  mission,
  task,
  run,
  onLaunch,
}: {
  mission: Mission
  task: Task | undefined
  run: Run | undefined
  onLaunch: (mission: Mission) => Promise<void>
}) {
  return (
    <article className="mission-card" data-testid={`mission-${mission.id}`}>
      <div className="mission-card-top">
        <span className={`status-chip status-chip-${mission.status}`}>{mission.status}</span>
        <span className="mission-id">#{shortId(mission.id)}</span>
      </div>
      <h3>{mission.title}</h3>
      <dl>
        <div>
          <dt>Task</dt>
          <dd>{task?.status ?? 'planning'}</dd>
        </div>
        <div>
          <dt>Run</dt>
          <dd>{run?.status ?? 'not started'}</dd>
        </div>
      </dl>
      {run?.artifact_sha256 ? (
        <div className="evidence-box">
          <strong>Verified artifact</strong>
          <span>{shortId(run.artifact_sha256)}…</span>
        </div>
      ) : null}
      {mission.status === 'ready' ? (
        <button className="button button-primary mission-launch" type="button" onClick={() => onLaunch(mission)}>
          Dispatch mission
        </button>
      ) : null}
    </article>
  )
}

function EventRow({ event, actors }: { event: DomainEvent; actors: Actor[] }) {
  const actor = actors.find((candidate) => candidate.id === event.actor_id)
  const label = event.type.replaceAll('.', ' / ')
  const detail =
    typeof event.payload.message === 'string'
      ? event.payload.message
      : typeof event.payload.summary === 'string'
        ? event.payload.summary
        : typeof event.payload.title === 'string'
          ? event.payload.title
          : event.aggregate_type

  return (
    <li className="event-row">
      <span className="event-seq">{String(event.seq).padStart(4, '0')}</span>
      <span className="event-type">{label}</span>
      <span className="event-detail">{detail}</span>
      <span className="event-actor">{actor?.name ?? 'system'}</span>
      <time dateTime={event.created_at}>{time(event.created_at)}</time>
    </li>
  )
}

function RoomPanel({
  room,
  messages,
  actors,
  selectedActor,
  missions,
  tasks,
  runs,
  onPost,
}: {
  room: { id: string; name: string; purpose: string } | undefined
  messages: RoomMessage[]
  actors: Actor[]
  selectedActor: Actor
  missions: Mission[]
  tasks: Task[]
  runs: Run[]
  onPost: (input: {
    roomId: string
    body: string
    replyToId: string | null
    mentions: string[]
    link: EntityLink | null
  }) => Promise<void>
}) {
  const [body, setBody] = useState('')
  const [replyToId, setReplyToId] = useState<string | null>(null)
  const [linkValue, setLinkValue] = useState('')

  if (!room) {
    return (
      <section className="room-panel panel">
        <div className="panel-heading">
          <div>
            <span className="section-code">ROOM / 03</span>
            <h2>No room access</h2>
            <p>{selectedActor.name} is not a member of this project room.</p>
          </div>
        </div>
        <div className="room-denied">Room messages and room-scoped work are hidden.</div>
      </section>
    )
  }

  const linkedOptions = [
    ...missions.slice(0, 2).map((mission) => ({
      value: `mission:${mission.id}`,
      label: `Mission · ${mission.title}`,
    })),
    ...tasks.slice(0, 2).map((task) => ({
      value: `task:${task.id}`,
      label: `Task · ${task.title}`,
    })),
    ...runs.slice(0, 2).flatMap((run) => [
      { value: `run:${run.id}`, label: `Run · ${shortId(run.id)} · ${run.status}` },
      ...(run.artifact_sha256
        ? [{ value: `artifact:${run.id}`, label: `Artifact · ${shortId(run.artifact_sha256)}` }]
        : []),
    ]),
  ]
  const visibleMessages = messages.slice(-40)
  const replyTarget = replyToId
    ? messages.find((message) => message.id === replyToId)
    : undefined

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!body.trim()) return
    const mentionedNames = Array.from(body.matchAll(/@([A-Za-z0-9_-]+)/g), (match) =>
      match[1].toLowerCase(),
    )
    const mentions = actors
      .filter((actor) => mentionedNames.includes(actor.name.toLowerCase()))
      .map((actor) => actor.id)
    const [kind, id] = linkValue.split(':')
    const link =
      kind && id
        ? ({ kind, id } as EntityLink)
        : null
    await onPost({
      roomId: room.id,
      body,
      replyToId,
      mentions,
      link,
    })
    setBody('')
    setReplyToId(null)
    setLinkValue('')
  }

  return (
    <section className="room-panel panel">
      <div className="panel-heading">
        <div>
          <span className="section-code">ROOM / 03</span>
          <h2>{room.name} wire</h2>
          <p>Humans and agents leave durable, linked messages here.</p>
        </div>
        <div className="room-count">{messages.length} messages</div>
      </div>
      <div className="room-layout">
        <ol className="room-message-list" data-testid="room-message-list">
          {visibleMessages.length ? (
            visibleMessages.map((message) => {
              const author = actors.find((actor) => actor.id === message.actor_id)
              const linked = message.link
                ? `${message.link.kind} · ${shortId(message.link.id)}`
                : null
              return (
                <li
                  key={message.id}
                  className={`room-message ${message.thread_root_id ? 'room-reply' : ''}`}
                >
                  <div className="room-message-head">
                    <strong>{author?.name ?? 'Unknown actor'}</strong>
                    <span>{author?.role ?? 'member'}</span>
                    <time dateTime={message.created_at}>{time(message.created_at)}</time>
                  </div>
                  <p>{message.body}</p>
                  <div className="room-message-foot">
                    <div>
                      {message.mentions.map((actorId) => {
                        const mentioned = actors.find((actor) => actor.id === actorId)
                        return (
                          <span className="mention-chip" key={actorId}>
                            @{mentioned?.name ?? shortId(actorId)}
                          </span>
                        )
                      })}
                      {linked ? <span className="entity-link">{linked}</span> : null}
                    </div>
                    <button type="button" onClick={() => setReplyToId(message.id)}>
                      Reply
                    </button>
                  </div>
                </li>
              )
            })
          ) : (
            <li className="empty-state">
              <strong>The room is quiet</strong>
              <span>Post the first durable message.</span>
            </li>
          )}
        </ol>
        <form className="room-composer" onSubmit={submit}>
          <label htmlFor="room-message">Post as {selectedActor.name}</label>
          {replyTarget ? (
            <div className="reply-context">
              Replying to {actors.find((actor) => actor.id === replyTarget.actor_id)?.name ?? 'message'}
              <button type="button" onClick={() => setReplyToId(null)}>Cancel</button>
            </div>
          ) : null}
          <textarea
            id="room-message"
            value={body}
            onChange={(event) => setBody(event.target.value)}
            placeholder="Write a message. Mention a colleague with @Name."
            rows={5}
          />
          <label htmlFor="room-link">Structured link</label>
          <select id="room-link" value={linkValue} onChange={(event) => setLinkValue(event.target.value)}>
            <option value="">No linked work item</option>
            {linkedOptions.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
          <button className="button button-primary" type="submit" disabled={!body.trim()}>
            Post to room
          </button>
        </form>
      </div>
    </section>
  )
}

function App() {
  const [bootstrap, setBootstrap] = useState<BootstrapResponse | null>(null)
  const [data, setData] = useState<SnapshotResponse | null>(null)
  const [selectedActorId, setSelectedActorId] = useState<string | null>(null)
  const [missionTitle, setMissionTitle] = useState(DEFAULT_MISSION)
  const [connection, setConnection] = useState<'connecting' | 'live' | 'offline'>('connecting')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [leaseTokens, setLeaseTokens] = useState<Record<string, string>>({})
  const reconnectTimer = useRef<number | null>(null)
  const lastEventSeq = useRef<Record<string, number>>({})

  const refresh = useCallback(async (corpId: string, actorId: string) => {
    const snapshot = await api<SnapshotResponse>(
      `/api/corps/${corpId}/snapshot?actor_id=${actorId}`,
    )
    const newest = snapshot.snapshot.events.at(-1)?.seq ?? 0
    lastEventSeq.current[actorId] = Math.max(lastEventSeq.current[actorId] ?? 0, newest)
    setData(snapshot)
  }, [])

  useEffect(() => {
    let cancelled = false
    void api<BootstrapResponse>('/api/demo/bootstrap', { method: 'POST', body: '{}' })
      .then(async (result) => {
        if (cancelled) return
        setBootstrap(result)
        const actorName = new URLSearchParams(window.location.search).get('actor')
        const initialActor = actorName?.toLowerCase() === 'bob'
          ? result.bob_actor_id
          : actorName?.toLowerCase() === 'eve'
            ? result.eve_actor_id
            : result.alice_actor_id
        setSelectedActorId(initialActor)
        await refresh(result.corp_id, initialActor)
      })
      .catch((caught: unknown) => setError(caught instanceof Error ? caught.message : String(caught)))
    return () => {
      cancelled = true
    }
  }, [refresh])

  useEffect(() => {
    if (!bootstrap || !selectedActorId) return
    let disposed = false
    let socket: WebSocket | null = null

    const connect = () => {
      if (disposed) return
      let replaying = true
      let replayChanged = false
      setConnection('connecting')
      const wsUrl = API_URL.replace(/^http/, 'ws')
      socket = new WebSocket(
        `${wsUrl}/ws/corps/${bootstrap.corp_id}?actor_id=${selectedActorId}&after_seq=${lastEventSeq.current[selectedActorId] ?? 0}`,
      )
      socket.onmessage = (event) => {
        const message = JSON.parse(event.data) as BrowserSocketMessage
        if (message.type === 'ready') {
          lastEventSeq.current[selectedActorId] = Math.max(
            lastEventSeq.current[selectedActorId] ?? 0,
            message.replayed_through,
          )
          replaying = false
          setConnection('live')
          if (replayChanged) void refresh(bootstrap.corp_id, selectedActorId)
          return
        }
        if (message.event.seq <= (lastEventSeq.current[selectedActorId] ?? 0)) return
        lastEventSeq.current[selectedActorId] = message.event.seq
        if (replaying) {
          replayChanged = true
          return
        }
        void refresh(bootstrap.corp_id, selectedActorId)
      }
      socket.onerror = () => setConnection('offline')
      socket.onclose = () => {
        if (disposed) return
        setConnection('offline')
        reconnectTimer.current = window.setTimeout(connect, 1_500)
      }
    }

    connect()
    return () => {
      disposed = true
      if (reconnectTimer.current !== null) window.clearTimeout(reconnectTimer.current)
      socket?.close()
    }
  }, [bootstrap, refresh, selectedActorId])

  const humans = useMemo(
    () => data?.snapshot.actors.filter((actor) => actor.kind === 'human') ?? [],
    [data],
  )
  const selectedActor =
    humans.find((actor) => actor.id === selectedActorId) ?? humans[0] ?? null
  const otherHuman = selectedActor
    ? humans.find((actor) => actor.id !== selectedActor.id)
    : undefined

  const selectActor = (actor: Actor) => {
    setData(null)
    setSelectedActorId(actor.id)
    const url = new URL(window.location.href)
    url.searchParams.set('actor', actor.name.toLowerCase())
    window.history.replaceState({}, '', url)
    if (bootstrap) {
      void refresh(bootstrap.corp_id, actor.id).catch((caught: unknown) => {
        setError(caught instanceof Error ? caught.message : String(caught))
      })
    }
  }

  const createMission = async (event: FormEvent) => {
    event.preventDefault()
    if (!bootstrap || !selectedActor || !missionTitle.trim()) return
    setBusy(true)
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/missions`, {
        method: 'POST',
        body: JSON.stringify({ title: missionTitle, requested_by: selectedActor.id }),
      })
      setMissionTitle('')
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const launchMission = async (mission: Mission) => {
    if (!bootstrap || !selectedActor) return
    setBusy(true)
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/missions/${mission.id}/launch`, {
        method: 'POST',
        body: JSON.stringify({ requested_by: selectedActor.id }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setBusy(false)
    }
  }

  const claimLease = async (agent: Agent) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      const result = await api<{
        acquired: boolean
        holder_actor_id: string
        token: string | null
      }>(
        `/api/corps/${bootstrap.corp_id}/agents/${agent.id}/lease`,
        {
          method: 'POST',
          body: JSON.stringify({ actor_id: selectedActor.id }),
        },
      )
      if (!result.acquired) {
        const holder = humans.find((actor) => actor.id === result.holder_actor_id)
        setError(`${holder?.name ?? 'Another operator'} currently controls ${agent.name}.`)
      } else if (result.token) {
        const key = leaseTokenKey(selectedActor.id, agent.id)
        setLeaseTokens((current) => ({ ...current, [key]: result.token as string }))
      }
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const releaseLease = async (agent: Agent, token: string) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/lease/release`, {
        method: 'POST',
        body: JSON.stringify({ actor_id: selectedActor.id, token }),
      })
      const key = leaseTokenKey(selectedActor.id, agent.id)
      setLeaseTokens((current) => {
        const next = { ...current }
        delete next[key]
        return next
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const transferLease = async (agent: Agent, token: string, toActor: Actor) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api<{ token: null; holder_actor_id: string }>(
        `/api/corps/${bootstrap.corp_id}/agents/${agent.id}/lease/transfer`,
        {
          method: 'POST',
          body: JSON.stringify({
            actor_id: selectedActor.id,
            token,
            to_actor_id: toActor.id,
          }),
        },
      )
      const fromKey = leaseTokenKey(selectedActor.id, agent.id)
      setLeaseTokens((current) => {
        const next = { ...current }
        delete next[fromKey]
        return next
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const emergencyStop = async (agent: Agent) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/emergency-stop`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          reason: `${selectedActor.name} requested an emergency stop from the operations floor.`,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const sendMessage = async (agent: Agent, text: string, token: string | undefined) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/agents/${agent.id}/messages`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          lease_token: token ?? null,
          text,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  const postRoomMessage = async (input: {
    roomId: string
    body: string
    replyToId: string | null
    mentions: string[]
    link: EntityLink | null
  }) => {
    if (!bootstrap || !selectedActor) return
    setError(null)
    try {
      await api(`/api/corps/${bootstrap.corp_id}/rooms/${input.roomId}/messages`, {
        method: 'POST',
        body: JSON.stringify({
          actor_id: selectedActor.id,
          body: input.body,
          reply_to_id: input.replyToId,
          mentions: input.mentions,
          link: input.link,
        }),
      })
      await refresh(bootstrap.corp_id, selectedActor.id)
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  if (!data || !bootstrap || !selectedActor) {
    return (
      <main className="loading-shell">
        <div className="loading-stamp">CRONY CORP</div>
        <h1>Opening the office ledger…</h1>
        {error ? <p className="error-banner">{error}</p> : <p>Waiting for the control plane.</p>}
      </main>
    )
  }

  const latestMissions = data.snapshot.missions.slice(0, 8)
  const latestEvents = data.snapshot.events.toReversed().slice(0, 28)
  const room = data.snapshot.rooms[0]
  const connectedRunners = data.runners.filter((runner) => runner.connected)
  const runnerLabel = connectedRunners.length
    ? `${connectedRunners.length} runner online`
    : data.runners.some((runner) => runner.status === 'grace')
      ? 'Runner reconnecting'
      : 'No runner'

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <span className="brand-kicker">Persistent operations office</span>
          <div className="brand-row">
            <span className="brand-mark">CC</span>
            <h1>Crony Corp</h1>
            <span className="alpha-stamp">ALPHA / SHIFT 00A</span>
          </div>
        </div>
        <div className="operator-console">
          <div className={`live-indicator live-${connection}`}>
            <span />
            {connection}
          </div>
          <div className={`runner-indicator ${connectedRunners.length ? 'runner-online' : ''}`}>
            {runnerLabel}
          </div>
          <label>
            Operating as
            <select value={selectedActor.id} onChange={(event) => {
              const actor = humans.find((candidate) => candidate.id === event.target.value)
              if (actor) selectActor(actor)
            }}>
              {humans.map((actor) => (
                <option key={actor.id} value={actor.id}>
                  {actor.name} · {actor.role}
                </option>
              ))}
            </select>
          </label>
        </div>
      </header>

      {error ? (
        <div className="error-banner" role="alert">
          <strong>Operations notice</strong>
          <span>{error}</span>
          <button type="button" onClick={() => setError(null)} aria-label="Dismiss error">
            ×
          </button>
        </div>
      ) : null}

      <section className="office-grid">
        <div className="floor-panel panel">
          <div className="panel-heading">
            <div>
              <span className="section-code">FLOOR / 01</span>
              <h2>{room?.name ?? 'Main floor'}</h2>
              <p>{room?.purpose}</p>
            </div>
            <div className="floor-legend">
              <span><StatusMark status="idle" /> idle</span>
              <span><StatusMark status="working" /> active</span>
              <span><StatusMark status="reviewing" /> review</span>
            </div>
          </div>
          <div className="floor-plan">
            <div className="corridor-label">AUTHORIZED STAFF BEYOND THIS LINE</div>
            {data.snapshot.agents.map((agent) => (
              <AgentDesk
                key={agent.id}
                agent={agent}
                actor={selectedActor}
                otherHuman={otherHuman}
                lease={data.snapshot.leases.find((lease) => lease.agent_id === agent.id)}
                leaseToken={leaseTokens[leaseTokenKey(selectedActor.id, agent.id)]}
                queuedCount={data.snapshot.queued_messages.filter((message) => message.agent_id === agent.id).length}
                onClaim={claimLease}
                onRelease={releaseLease}
                onTransfer={transferLease}
                onEmergencyStop={emergencyStop}
                onMessage={sendMessage}
              />
            ))}
          </div>
        </div>

        <aside className="mission-panel panel">
          <div className="panel-heading">
            <div>
              <span className="section-code">MISSIONS / 02</span>
              <h2>Dispatch ledger</h2>
              <p>Work is only complete when evidence lands.</p>
            </div>
          </div>
          <form className="mission-form" onSubmit={createMission}>
            <label htmlFor="mission-title">New mission</label>
            <textarea
              id="mission-title"
              value={missionTitle}
              onChange={(event) => setMissionTitle(event.target.value)}
              placeholder="Describe the outcome the Corp should produce."
              rows={4}
            />
            <button className="button button-primary" type="submit" disabled={busy || !missionTitle.trim()}>
              File mission
            </button>
          </form>
          <div className="mission-list">
            {latestMissions.length ? (
              latestMissions.map((mission) => {
                const task = data.snapshot.tasks.find((candidate) => candidate.mission_id === mission.id)
                const run = task ? data.snapshot.runs.find((candidate) => candidate.task_id === task.id) : undefined
                return (
                  <MissionCard
                    key={mission.id}
                    mission={mission}
                    task={task}
                    run={run}
                    onLaunch={launchMission}
                  />
                )
              })
            ) : (
              <div className="empty-state">
                <strong>No missions filed</strong>
                <span>Write the first outcome above.</span>
              </div>
            )}
          </div>
        </aside>
      </section>

      <RoomPanel
        room={room}
        messages={data.snapshot.room_messages}
        actors={data.snapshot.actors}
        selectedActor={selectedActor}
        missions={data.snapshot.missions}
        tasks={data.snapshot.tasks}
        runs={data.snapshot.runs}
        onPost={postRoomMessage}
      />

      <section className="operations-panel panel">
        <div className="panel-heading operations-heading">
          <div>
            <span className="section-code">JOURNAL / 04</span>
            <h2>Immutable activity</h2>
          </div>
          <div className="operations-summary">
            <span>{data.snapshot.missions.length} missions</span>
            <span>{data.snapshot.runs.length} runs</span>
            <span>{data.snapshot.events.length} events loaded</span>
          </div>
        </div>
        <ol className="event-list" data-testid="event-list">
          {latestEvents.map((event) => (
            <EventRow key={event.id} event={event} actors={data.snapshot.actors} />
          ))}
        </ol>
      </section>
    </main>
  )
}

export default App
