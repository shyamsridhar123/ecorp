import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import type { RepositoryTarget, RunnerNode } from './missionRuntime'
import {
  CODING_AGENTS, connectionLabel, connectionRunnerRevision, connectionScope,
  connectionStatusLabel, connectionsNeedPresenceRefresh, pendingSetup, signInUrl,
} from './workspaceConnections'
import type {
  CodingAgent, ConnectionRepository, GitHubRepositoryChoice, SetupOperation,
  SetupResponse, WorkspaceConnection, WorkspaceConnections,
} from './workspaceConnections'
import './ConnectionsPanel.css'

type ApiClient = <T>(path: string, init?: RequestInit) => Promise<T>
type Props = {
  corpId: string
  roomId: string
  actorId: string
  actorRole: string
  runners: RunnerNode[]
  initialSource?: RepositoryTarget
  api: ApiClient
  refreshRevision?: number
  onSelect: (connection: WorkspaceConnection) => void
  onClose: () => void
}

const empty: WorkspaceConnections = { connections: [], operations: [], selected_connection_id: null }

export function ConnectionsPanel({
  corpId, roomId, actorId, actorRole, runners, initialSource, api, refreshRevision = 0, onSelect, onClose,
}: Props) {
  const scope = connectionScope(corpId, roomId, actorId)
  const scopeRef = useRef(scope)
  useLayoutEffect(() => { scopeRef.current = scope }, [scope])
  const currentRunners = useRef(runners)
  useLayoutEffect(() => { currentRunners.current = runners }, [runners])
  const runnerRevision = connectionRunnerRevision(runners)
  const mounted = useRef(false)
  const dialog = useRef<HTMLDialogElement>(null)
  const canManageMachine = ['owner', 'admin'].includes(actorRole)
  const [data, setData] = useState<WorkspaceConnections>(empty)
  const [operation, setOperation] = useState<SetupOperation | null>(null)
  const [runnerId, setRunnerId] = useState(
    initialSource?.runnerIds[0] ?? runners.find((runner) => runner.connected)?.id ?? runners[0]?.id ?? '',
  )
  const [sourceKind, setSourceKind] = useState<'github' | 'local' | 'advertised'>(
    initialSource && !initialSource.workspaceConnectionId ? 'advertised' : 'github',
  )
  const [repository, setRepository] = useState('')
  const [directory, setDirectory] = useState('')
  const [baseRef, setBaseRef] = useState('HEAD')
  const [label, setLabel] = useState('')
  const [agent, setAgent] = useState<CodingAgent>('github-copilot')
  const [systemInstallation, setSystemInstallation] = useState(false)
  const [machineAccount, setMachineAccount] = useState(canManageMachine)
  const [account, setAccount] = useState<'machine' | 'personal'>(canManageMachine ? 'machine' : 'personal')
  const [repositories, setRepositories] = useState<GitHubRepositoryChoice[]>([])
  const [accountLogin, setAccountLogin] = useState<string | null>(null)
  const [authorizationCode, setAuthorizationCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const requestKey = useRef<{ body: string; key: string } | null>(null)
  const selectedRunner = runners.find((runner) => runner.id === runnerId)
  const setupAvailable = selectedRunner?.capabilities.some(
    (capability) => capability.name === 'workspace-setup-v1' && capability.available,
  ) ?? false
  const base = `/api/corps/${corpId}/rooms/${roomId}/connections`
  const liveOperation = operation && pendingSetup(operation)
  const operationId = operation?.id
  const operationPending = Boolean(liveOperation)
  const instruction = operation?.report?.sign_in
  const verificationUrl = instruction ? signInUrl(instruction) : undefined
  const candidates = useMemo(() => {
    const query = repository.trim().toLowerCase()
    return repositories.filter((candidate) => candidate.repository.toLowerCase().includes(query)).slice(0, 40)
  }, [repositories, repository])

  const isCurrent = useCallback((expected: string) =>
    mounted.current && scopeRef.current === expected, [])

  useEffect(() => {
    mounted.current = true
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const element = dialog.current
    element?.showModal()
    return () => {
      mounted.current = false
      element?.close()
      if (opener?.isConnected) opener.focus()
    }
  }, [])

  const refresh = useCallback(async () => {
    const expected = scope
    const next = await api<WorkspaceConnections>(`${base}?actor_id=${encodeURIComponent(actorId)}`)
    if (isCurrent(expected)) setData(next)
    return next
  }, [api, actorId, base, scope, isCurrent])

  useEffect(() => {
    let current = true
    let timer: ReturnType<typeof setTimeout> | undefined
    let attempts = 0
    const load = async () => {
      attempts += 1
      try {
        const next = await api<WorkspaceConnections>(`${base}?actor_id=${encodeURIComponent(actorId)}`)
        if (!current || !isCurrent(scope)) return
        setData(next)
        const active = next.operations.find((candidate) => pendingSetup(candidate))
        if (active) {
          setOperation(active)
          setRunnerId(active.runner_id)
        }
        if (attempts < 5 && connectionsNeedPresenceRefresh(next.connections, currentRunners.current)) {
          timer = setTimeout(() => void load(), 1000)
        }
      } catch (reason) {
        if (current && isCurrent(scope)) {
          setError(reason instanceof Error ? reason.message : 'Could not refresh the saved connections.')
        }
      }
    }
    void load()
    return () => {
      current = false
      if (timer !== undefined) clearTimeout(timer)
    }
  }, [api, base, actorId, scope, isCurrent, runnerRevision, refreshRevision])

  useEffect(() => {
    if (!operationId || !operationPending) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      try {
        const next = await api<SetupOperation>(
          `/api/corps/${corpId}/setup-operations/${operationId}?actor_id=${encodeURIComponent(actorId)}`,
        )
        if (cancelled || !isCurrent(scope)) return
        setOperation(next)
        if (next.report?.account_login) setAccountLogin(next.report.account_login)
        if (next.kind === 'list_github_repositories' && next.status === 'succeeded') {
          setRepositories(next.report?.repositories ?? [])
        }
        if (!pendingSetup(next)) {
          await refresh()
          return
        }
      } catch (reason) {
        if (!cancelled && isCurrent(scope)) {
          setError(reason instanceof Error ? reason.message : 'Could not refresh the connection check.')
        }
      }
      if (!cancelled) timer = setTimeout(poll, 1500)
    }
    timer = setTimeout(poll, 700)
    return () => { cancelled = true; clearTimeout(timer) }
  }, [api, actorId, corpId, operationId, operationPending, refresh, scope, isCurrent])

  async function send(path: string, body: object) {
    const expected = scope
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const response = await api<SetupResponse>(path, {
        method: 'POST', body: JSON.stringify({ actor_id: actorId, ...body }),
      })
      if (!isCurrent(expected)) return
      setOperation(response.operation)
      if (response.operation.report?.account_login) setAccountLogin(response.operation.report.account_login)
      if (response.operation.kind === 'list_github_repositories' && response.operation.status === 'succeeded') {
        setRepositories(response.operation.report?.repositories ?? [])
      }
      await refresh()
    } catch (reason) {
      if (isCurrent(expected)) setError(reason instanceof Error ? reason.message : 'The connection could not be started.')
    } finally {
      if (isCurrent(expected)) setBusy(false)
    }
  }

  function github(action: 'inspect' | 'sign_in' | 'repositories') {
    if (action === 'sign_in') {
      setAccount('personal')
      setAccountLogin(null)
      setRepositories([])
    }
    void send(`${base}/github`, {
      runner_id: runnerId, account: action === 'sign_in' ? 'personal' : account,
      action, idempotency_key: crypto.randomUUID(),
    })
  }

  function connect(event: FormEvent) {
    event.preventDefault()
    let source: ConnectionRepository
    if (sourceKind === 'advertised' && initialSource) {
      source = { kind: 'advertised', source: {
        repository: initialSource.repository, base_ref: initialSource.baseRef,
        base_commit: initialSource.baseCommit,
      } }
    } else if (sourceKind === 'local') {
      source = { kind: 'local', directory: directory.trim(), base_ref: baseRef.trim() || 'HEAD' }
    } else {
      const candidate = repositories.find((entry) =>
        entry.repository.toLowerCase() === repository.trim().toLowerCase())
      source = {
        kind: 'github', repository: repository.trim(),
        ...(candidate ? { repository_id: candidate.id } : {}),
        base_ref: baseRef.trim() || 'HEAD', account,
      }
    }
    const sourceName = sourceKind === 'local' ? directory.split(/[\\/]/).filter(Boolean).at(-1)
      : sourceKind === 'advertised' ? initialSource?.repository : repository
    const body = {
      runner_id: runnerId,
      label: label.trim() || `${sourceName || 'Project'} · ${connectionLabel(agent)}`.slice(0, 110),
      configuration: {
        repository: source, agent, use_system_installation: systemInstallation && canManageMachine,
        use_machine_account: machineAccount && canManageMachine,
      },
    }
    const serialized = JSON.stringify(body)
    if (requestKey.current?.body !== serialized) {
      requestKey.current = { body: serialized, key: crypto.randomUUID() }
    }
    void send(base, { ...body, idempotency_key: requestKey.current.key })
  }

  async function selectConnection(connection: WorkspaceConnection) {
    const expected = scope
    setBusy(true)
    setError('')
    try {
      await api(`${base}/selection`, {
        method: 'POST', body: JSON.stringify({ actor_id: actorId, connection_id: connection.id }),
      })
      if (isCurrent(expected)) onSelect(connection)
    } catch (reason) {
      if (isCurrent(expected)) setError(reason instanceof Error ? reason.message : 'Could not remember this connection.')
    } finally {
      if (isCurrent(expected)) setBusy(false)
    }
  }

  async function submitCode(event: FormEvent) {
    event.preventDefault()
    if (!operation || !instruction?.input_id || !authorizationCode.trim()) return
    const code = authorizationCode.trim()
    const expected = scope
    setAuthorizationCode('')
    setBusy(true)
    setError('')
    try {
      await api(`/api/corps/${corpId}/setup-operations/${operation.id}/sign-in-response`, {
        method: 'POST', body: JSON.stringify({
          actor_id: actorId, input_id: instruction.input_id, authorization_code: code,
        }),
      })
      if (isCurrent(expected)) setNotice('Code submitted to the native sign-in. Waiting for confirmation.')
    } catch (reason) {
      if (isCurrent(expected)) setError(reason instanceof Error ? reason.message : 'The sign-in step did not confirm the code.')
    } finally {
      if (isCurrent(expected)) setBusy(false)
    }
  }

  return (
    <dialog ref={dialog} className="connections-dialog" aria-labelledby="connections-title"
      onCancel={(event) => { event.preventDefault(); onClose() }}>
      <header className="connections-header">
        <div>
          <h2 id="connections-title">Connect a project</h2>
          <p>Choose where ECorp works and the coding agent it uses. Set it up once, then get on with the work.</p>
        </div>
        <button className="button button-secondary" type="button" onClick={onClose}>Close</button>
      </header>

      {error && <div className="connections-message connections-error" role="alert">{error.replace(/^forbidden:\s*/i, '')}</div>}
      {notice && <p className="connections-message" role="status">{notice}</p>}

      {operation && (
        <section className="connections-progress" aria-label="Current connection check" aria-live="polite">
          <div className="connections-progress-heading">
            <strong>{pendingSetup(operation) ? 'Connection in progress' : operation.status === 'succeeded' ? 'Check finished' : 'Connection needs attention'}</strong>
            <span>{operation.report?.connection_status?.replaceAll('_', ' ') ?? operation.status.replaceAll('_', ' ')}</span>
          </div>
          <p>{operation.report?.detail ?? 'Waiting for the selected machine. This continues if you close the panel.'}</p>
          {instruction && (
            <div className="connections-signin">
              <p>Sign in directly with {connectionLabel(instruction.provider)}. ECorp does not ask for your password.</p>
              {instruction.user_code && (
                <div className="connections-device-code">
                  <span>Enter this code on the sign-in page</span>
                  <code>{instruction.user_code}</code>
                  <button className="button button-secondary" type="button" onClick={() => {
                    void navigator.clipboard.writeText(instruction.user_code ?? '').then(
                      () => setNotice('Sign-in code copied.'),
                      () => setError('Select and copy the code above.'),
                    )
                  }}>Copy code</button>
                </div>
              )}
              {verificationUrl
                ? <a className="button button-primary" href={verificationUrl} target="_blank" rel="noopener noreferrer">Open sign-in page</a>
                : <p role="alert">The sign-in address was not recognized. Check the native agent configuration.</p>}
              {instruction.input_kind === 'authorization_code' && instruction.input_id && (
                <form className="connections-code-form" onSubmit={(event) => { void submitCode(event) }}>
                  <label htmlFor="native-authorization-code">One-time code from the sign-in page</label>
                  <input id="native-authorization-code" type="password" autoComplete="one-time-code"
                    value={authorizationCode} onChange={(event) => setAuthorizationCode(event.target.value)}
                    maxLength={4096} />
                  <button className="button button-primary" disabled={busy || !authorizationCode.trim()}>Complete sign-in</button>
                </form>
              )}
              <small>Only your operator can see this sign-in step. The native account stays on the selected machine.</small>
            </div>
          )}
          {operation.connection_id && operation.report?.connection_status === 'needs_sign_in' && !pendingSetup(operation) && (
            <button className="button button-primary" type="button" disabled={busy} onClick={() => {
              void send(`/api/corps/${corpId}/connections/${operation.connection_id}/sign-in`, { idempotency_key: crypto.randomUUID() })
            }}>Sign in to the coding agent</button>
          )}
        </section>
      )}

      {data.connections.length > 0 && (
        <section className="connections-saved" aria-labelledby="saved-connections-title">
          <h3 id="saved-connections-title">Saved connections</h3>
          <ul>
            {data.connections.map((connection) => (
              <li key={connection.id}>
                <div className="connections-saved-title">
                  <strong>{connection.label}</strong>
                  <span>{connection.source?.repository ?? 'Repository setup pending'} · {connectionLabel(connection.agent)}</span>
                </div>
                <span className={`connection-state connection-state-${connection.status}`}>{connectionStatusLabel(connection)}</span>
                <div className="connections-saved-actions">
                  <button className="button button-primary" type="button" disabled={busy || !connection.source}
                    onClick={() => { void selectConnection(connection) }}>Use</button>
                  <button className="button button-secondary" type="button" disabled={busy || !connection.runner_connected}
                    onClick={() => { void send(`/api/corps/${corpId}/connections/${connection.id}/check`, { idempotency_key: crypto.randomUUID() }) }}>Test</button>
                </div>
              </li>
            ))}
          </ul>
          <p className="connections-help">Offline connections stay saved. Starting work still requires the selected machine to be connected.</p>
        </section>
      )}

      <form className="connections-form" onSubmit={connect}>
        <h3>{data.connections.length ? 'Connect another project or agent' : 'Your first connection'}</h3>
        <label htmlFor="connection-machine">Execution machine</label>
        <select id="connection-machine" value={runnerId} onChange={(event) => {
          setRunnerId(event.target.value); setRepositories([]); setAccountLogin(null)
        }} disabled={busy || Boolean(liveOperation)}>
          {runners.map((runner) => <option key={runner.id} value={runner.id}>
            {runner.hostname} · {runner.id}{runner.connected ? '' : ' · offline'}
          </option>)}
        </select>
        {!runners.length && <p className="connections-help">Connect an ECorp runner to this workspace first.</p>}
        {selectedRunner && !setupAvailable && <p className="connections-help">This machine needs a runner that supports project connections. Existing missions are unaffected.</p>}

        <fieldset className="connections-source-choice">
          <legend>Repository</legend>
          <label><input type="radio" name="connection-source" checked={sourceKind === 'github'} onChange={() => setSourceKind('github')} /> GitHub repository</label>
          {canManageMachine && <label><input type="radio" name="connection-source" checked={sourceKind === 'local'} onChange={() => setSourceKind('local')} /> Existing local checkout</label>}
          {initialSource && !initialSource.workspaceConnectionId && <label><input type="radio" name="connection-source" checked={sourceKind === 'advertised'} onChange={() => setSourceKind('advertised')} /> Save the current repository</label>}
        </fieldset>
        {sourceKind === 'github' && <>
          <div className="connections-github-account">
            <span>{accountLogin ? `GitHub: ${accountLogin}` : account === 'machine' ? 'Use the machine’s configured GitHub account' : 'Use your GitHub account on this machine'}</span>
            <button className="button button-secondary" type="button" disabled={busy || Boolean(liveOperation) || !setupAvailable}
              onClick={() => github('repositories')}>Browse repositories</button>
            <button className="button button-secondary" type="button" disabled={busy || Boolean(liveOperation) || !setupAvailable}
              onClick={() => github('sign_in')}>Sign in with GitHub</button>
          </div>
          <label htmlFor="connection-repository">Repository name or GitHub URL</label>
          <input id="connection-repository" value={repository} onChange={(event) => setRepository(event.target.value)}
            placeholder="your-team/your-application" autoComplete="off" required maxLength={500} />
          {repositories.length > 0 && <ul className="connections-repository-results" aria-label="Matching GitHub repositories">
            {candidates.map((candidate) => <li key={candidate.id}><button type="button" onClick={() => {
              setRepository(candidate.repository); setBaseRef(candidate.default_branch)
            }}><span>{candidate.repository}</span><small>{candidate.private ? 'Private' : 'Public'}</small></button></li>)}
            {!candidates.length && <li>No match in this list. You can still enter an authorized repository URL.</li>}
          </ul>}
        </>}
        {sourceKind === 'local' && <>
          <label htmlFor="connection-directory">Repository folder on {selectedRunner?.hostname ?? 'the selected machine'}</label>
          <input id="connection-directory" value={directory} onChange={(event) => setDirectory(event.target.value)}
            placeholder="C:\\projects\\your-application" autoComplete="off" required maxLength={2048} />
          <p className="connections-help">Use an existing Git checkout in one of this machine’s approved source locations. Agent changes will go into separate worktrees.</p>
        </>}
        {sourceKind === 'advertised' && initialSource && <p className="connections-current-source">
          <strong>{initialSource.repository}</strong><span>{initialSource.baseRef} · {initialSource.baseCommit.slice(0, 12)}</span>
        </p>}

        <fieldset className="connections-agent-choice">
          <legend>Coding agent</legend>
          {CODING_AGENTS.map((candidate) => <label key={candidate.id}>
            <input type="radio" name="connection-agent" value={candidate.id} checked={agent === candidate.id}
              onChange={() => {
                setAgent(candidate.id)
                setSystemInstallation(canManageMachine && !selectedRunner?.capabilities.some(
                  (capability) => capability.name === candidate.id && !capability.workspace_connection_id && capability.available,
                ))
                setMachineAccount(canManageMachine && selectedRunner?.capabilities.some(
                  (capability) => capability.name === candidate.id && !capability.workspace_connection_id && capability.available,
                ) === true)
              }} />
            <strong>{candidate.label}</strong>
          </label>)}
        </fieldset>
        <p className="connections-help">ECorp checks this agent’s actual installation, sign-in and native connection. Selecting a name alone does not make it ready.</p>
        <details className="connections-options">
          <summary>Connection options</summary>
          <label htmlFor="connection-label">Connection name <span>(optional)</span></label>
          <input id="connection-label" value={label} onChange={(event) => setLabel(event.target.value)} maxLength={100} />
          {sourceKind !== 'advertised' && <>
            <label htmlFor="connection-ref">Source branch or ref</label>
            <input id="connection-ref" value={baseRef} onChange={(event) => setBaseRef(event.target.value)} maxLength={240} />
          </>}
          {sourceKind === 'github' && canManageMachine && <>
            <label htmlFor="github-account-source">GitHub connection</label>
            <select id="github-account-source" value={account} onChange={(event) => {
              setAccount(event.target.value as 'machine' | 'personal'); setAccountLogin(null); setRepositories([])
            }}>
              <option value="machine">Machine’s configured account</option>
              <option value="personal">Your isolated native account configuration</option>
            </select>
          </>}
          {canManageMachine && <label className="connections-checkbox">
            <input type="checkbox" checked={systemInstallation} onChange={(event) => setSystemInstallation(event.target.checked)} />
            Use the machine’s standard agent installation
          </label>}
          {canManageMachine && <label className="connections-checkbox">
            <input type="checkbox" checked={machineAccount} onChange={(event) => setMachineAccount(event.target.checked)} />
            Use the agent’s existing sign-in on this machine
          </label>}
          {!machineAccount && <p className="connections-help">Your native sign-in is kept in a separate connection profile. Choosing an installed agent does not reuse someone else’s account.</p>}
        </details>
        <footer className="connections-form-footer">
          <p>Saved for this project room. Sign-in details are private to your operator.</p>
          <button className="button button-primary" disabled={busy || Boolean(liveOperation) || !runnerId || !setupAvailable}>
            {busy ? 'Starting connection…' : 'Connect and test'}
          </button>
        </footer>
      </form>
    </dialog>
  )
}
