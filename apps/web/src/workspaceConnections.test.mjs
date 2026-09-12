import assert from 'node:assert/strict'
import test from 'node:test'
import {
  chooseSavedConnection, connectionRunnerRevision, connectionRuntime, connectionScope,
  connectionsNeedPresenceRefresh, connectionStatusLabel, connectionTarget, signInUrl,
} from './workspaceConnections.ts'

const connection = {
  id: 'connection-a', corp_id: 'corp', room_id: 'room', created_by: 'alice',
  runner_id: 'machine', label: 'Team app', agent: 'codex',
  source: { repository: 'team/app', base_ref: 'main', base_commit: 'a'.repeat(40) },
  status: 'ready', detail: 'Native connection checked', models: [],
  version: 1, last_checked_at: null, runner_connected: true,
  created_at: '', updated_at: '',
}
const capabilities = [
  { name: 'workspace-isolation', available: true, workspace_connection_id: connection.id,
    source_repository: 'team/app', source_base_ref: 'main', source_base_commit: 'a'.repeat(40) },
  { name: 'codex', available: true, workspace_connection_id: connection.id, models: [] },
]
const runner = {
  id: 'machine', corp_id: 'corp', connected: true, status: 'connected',
  hostname: 'workstation', os: 'windows', capabilities,
}

function deepFreeze(value) {
  if (value && typeof value === 'object') {
    for (const child of Object.values(value)) deepFreeze(child)
    Object.freeze(value)
  }
  return value
}

test('runner revision is stable for reordered equivalent records without sorting the input', () => {
  const runners = deepFreeze(structuredClone([
    runner,
    { ...runner, id: 'another-machine' },
    { ...runner, corp_id: 'another-corp' },
    { ...runner, connected: false, status: 'grace' },
  ]))
  const before = structuredClone(runners)
  const revision = connectionRunnerRevision(runners)
  assert.equal(revision, connectionRunnerRevision([...runners].reverse()))
  assert.equal(revision, connectionRunnerRevision(structuredClone(runners)))
  assert.equal(connectionRunnerRevision([]), '[]')
  assert.deepEqual(runners, before)
})

test('runner revision ignores heartbeat, display and capability churn', () => {
  const before = {
    ...runner, last_seen_at: '2026-09-09T04:00:00Z', grace_expires_at: null,
  }
  const after = {
    ...before, last_seen_at: '2026-09-09T04:00:05Z',
    grace_expires_at: '2026-09-09T04:01:00Z', hostname: 'renamed', os: 'linux',
    capabilities: capabilities.map((capability) => ({ ...capability, available: false })),
  }
  assert.equal(connectionRunnerRevision([before]), connectionRunnerRevision([after]))
})

test('runner revision changes only when lifecycle identity or presence records change', () => {
  const revision = connectionRunnerRevision([runner])
  for (const change of [
    { id: 'another-machine' }, { corp_id: 'another-corp' },
    { connected: false }, { status: 'grace' }, { status: 'offline' },
  ]) {
    assert.notEqual(connectionRunnerRevision([{ ...runner, ...change }]), revision)
  }
  assert.notEqual(connectionRunnerRevision([]), revision)
  assert.notEqual(connectionRunnerRevision([runner, { ...runner, id: 'new-machine' }]), revision)
})

test('offline presence requests revalidation without rewriting endpoint readiness', () => {
  for (const status of ['grace', 'offline']) {
    const observed = [{ ...runner, connected: false, status }]
    assert.equal(connectionsNeedPresenceRefresh([connection], observed), true)
    assert.equal(connection.runner_connected, true)
    assert.equal(connection.status, 'ready')
    assert.equal(connectionsNeedPresenceRefresh([
      { ...connection, runner_connected: false },
    ], observed), false)
  }
  assert.equal(connectionsNeedPresenceRefresh([], [runner]), false)
})

test('reconnect mismatch remains a hint until the endpoint itself returns ready presence', () => {
  const endpoint = deepFreeze({ ...connection, runner_connected: false })
  assert.equal(connectionsNeedPresenceRefresh([endpoint], [
    { ...runner, connected: false, status: 'offline' },
  ]), false)
  // Persisted connected presence can precede dispatch_ready; repeated hints must
  // not make the authoritative endpoint flag true before reconciliation finishes.
  assert.equal(connectionsNeedPresenceRefresh([endpoint], [runner]), true)
  assert.equal(connectionsNeedPresenceRefresh([endpoint], [runner]), true)
  assert.equal(endpoint.runner_connected, false)
  assert.equal(connectionStatusLabel(endpoint), 'Machine offline')
  assert.equal(connectionsNeedPresenceRefresh([
    { ...endpoint, runner_connected: true },
  ], [runner]), false)
  assert.equal(connectionsNeedPresenceRefresh([
    connection, { ...endpoint, id: 'another-connection' },
  ], [runner]), true)
})

test('unknown presence is false and same-ID foreign runners cannot supply presence', () => {
  const endpointOffline = { ...connection, runner_connected: false }
  for (const observed of [
    [],
    [{ ...runner, corp_id: undefined }],
    [{ ...runner, corp_id: 'another-corp' }],
    [{ ...runner, id: 'another-machine' }],
    [{ ...runner, connected: undefined }],
  ]) {
    assert.equal(connectionsNeedPresenceRefresh([connection], observed), true)
    assert.equal(connectionsNeedPresenceRefresh([endpointOffline], observed), false)
  }
  for (const connected of [false, true]) {
    const observed = [
      { ...runner, corp_id: 'another-corp', connected: !connected },
      { ...runner, connected },
    ]
    assert.equal(connectionsNeedPresenceRefresh([connection], observed), !connected)
    assert.equal(connectionsNeedPresenceRefresh([endpointOffline], observed), connected)
  }
})

test('presence hints never promote or erase native sign-in and installation states', () => {
  for (const [status, label] of [
    ['needs_sign_in', 'Sign-in needed'],
    ['not_installed', 'Agent not installed'],
    ['incompatible', 'Setup needs attention'],
    ['failed', 'Check failed'],
  ]) {
    for (const connected of [false, true]) {
      const endpoint = deepFreeze({ ...connection, status, runner_connected: !connected })
      const before = structuredClone(endpoint)
      assert.equal(connectionsNeedPresenceRefresh([endpoint], [{ ...runner, connected }]), true)
      assert.equal(connectionStatusLabel(endpoint), label)
      assert.equal(connectionRuntime(endpoint, [runner]), undefined)
      assert.deepEqual(endpoint, before)
    }
  }
})

test('revision and presence helpers leave source, models, identity and selection immutable', () => {
  const state = deepFreeze(structuredClone({
    connections: [{
      ...connection,
      source: { ...connection.source, repository_id: 'repository-identity' },
      models: [{
        id: 'native-model', name: 'Native model', policy_state: 'disabled',
        policy_terms: 'Native policy', supports_vision: true, supports_reasoning_effort: true,
        max_prompt_tokens: 1000, max_context_window_tokens: 2000,
        supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium',
        billing_multiplier: 1,
      }],
      runner_connected: false, version: 7, last_checked_at: '2026-09-09T04:00:00Z',
      created_at: '2026-09-08T20:00:00Z', updated_at: '2026-09-09T04:00:00Z',
    }, { ...connection, id: 'unchecked', status: 'connecting', source: null }],
    selected_connection_id: connection.id,
    runners: [runner],
  }))
  const before = structuredClone(state)
  const saved = chooseSavedConnection(state.connections, state.selected_connection_id)
  const target = connectionTarget(saved, state.runners)
  assert.equal(typeof connectionRunnerRevision(state.runners), 'string')
  assert.equal(connectionsNeedPresenceRefresh(state.connections, state.runners), true)
  assert.equal(saved.runner_connected, false)
  assert.equal(saved.models[0].policy_state, 'disabled')
  assert.equal(state.connections[1].source, null)
  assert.equal(connectionTarget(state.connections[1], state.runners), undefined)
  assert.deepEqual(connectionTarget(saved, state.runners), target)
  assert.equal(chooseSavedConnection(state.connections, state.selected_connection_id), saved)
  assert.equal(chooseSavedConnection(state.connections, null), undefined)
  assert.deepEqual(state, before)
})

test('saved choice and source remain present while its machine is offline', () => {
  const offline = { ...connection, runner_connected: false }
  assert.equal(chooseSavedConnection([offline], connection.id), offline)
  assert.equal(connectionTarget(offline, []).repository, 'team/app')
  assert.equal(connectionStatusLabel(offline), 'Machine offline')
  assert.equal(connectionRuntime(offline, [{ ...runner, connected: false }]), undefined)
  assert.equal(chooseSavedConnection([offline], 'missing'), undefined)
})

test('ready metadata alone cannot borrow another connection or source for dispatch', () => {
  assert.equal(connectionRuntime(connection, [runner]).name, 'codex')
  assert.equal(connectionRuntime(connection, [{
    ...runner, capabilities: capabilities.map((cap) => ({ ...cap, workspace_connection_id: 'another' })),
  }]), undefined)
  assert.equal(connectionRuntime(connection, [{
    ...runner, capabilities: capabilities.map((cap) =>
      cap.name === 'workspace-isolation' ? { ...cap, source_base_commit: 'b'.repeat(40) } : cap),
  }]), undefined)
  assert.equal(connectionRuntime({ ...connection, status: 'needs_sign_in' }, [runner]), undefined)
})

test('connection UI identity is scoped to actor and room', () => {
  assert.notEqual(connectionScope('corp', 'room', 'alice'), connectionScope('corp', 'room', 'bob'))
  assert.notEqual(connectionScope('corp', 'room', 'alice'), connectionScope('corp', 'other', 'alice'))
})

test('native sign-in instructions cannot navigate to another host or credential-bearing URL', () => {
  assert.equal(signInUrl({ provider: 'github', verification_uri: 'https://github.com/login/device' }),
    'https://github.com/login/device')
  for (const verification_uri of ['javascript:alert(1)', 'https://github.com.evil/login',
    'https://secret@github.com/login/device', 'http://github.com/login/device']) {
    assert.equal(signInUrl({ provider: 'github', verification_uri }), undefined)
  }
})
