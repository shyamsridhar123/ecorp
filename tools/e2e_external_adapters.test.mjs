import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import {
  assertExternalRunner,
  assertUnavailableLaunch,
  externalAdapterConfig,
  runExternalAdapterContract,
} from './e2e_external_adapters.mjs'

const env = { CRONY_EXTERNAL_ADAPTER_TEST: '1', CRONY_SERVER_HTTP: 'http://127.0.0.1:18437' }
const disabledSummary = 'spawn=no, stream=no, steer=no, interrupt=no, stop=no, resume=no, usage=no, artifacts=no'
const enabledSummary = 'spawn=yes, stream=yes, steer=no, interrupt=yes, stop=yes, resume=yes, usage=yes, artifacts=yes'
const providers = ['claude-code', 'opencode']
const runner = (os, available = os === 'windows') => ({
  id: 'fixture-runner', os, connected: true,
  capabilities: [
    ...providers.map(name => ({ name, available, detail: name + '; ' + (os === 'windows' ? enabledSummary : disabledSummary) })),
    { name: 'workspace-isolation', available: true, source_repository: 'local/fixture-123',
      source_base_ref: 'HEAD', source_base_commit: 'a'.repeat(40) },
  ],
})

test('fixture configuration requires explicit ownership, origin and one platform expectation', () => {
  assert.equal(externalAdapterConfig(['--expect-unix', '--dry-run'], env).dryRun, true)
  assert.equal(externalAdapterConfig(['--expect-windows'], env).expectedPlatform, 'windows')
  for (const args of [[], ['--expect-unix', '--expect-windows'], ['--expect-unix', '--expect-unix'], ['--skip']]) {
    assert.throws(() => externalAdapterConfig(args, env))
  }
  assert.throws(() => externalAdapterConfig(['--expect-unix'], {}), /owned disposable fixture/)
  assert.throws(() => externalAdapterConfig(['--expect-unix'], { CRONY_EXTERNAL_ADAPTER_TEST: '1' }), /explicit owned/)
})

test('manual, remote, credential-bearing and non-origin endpoints are rejected', () => {
  for (const server of [
    'http://127.0.0.1:8791', 'http://127.0.0.1:8793', 'https://example.com:18437',
    'http://user:secret@127.0.0.1:18437', 'http://127.0.0.1:18437/path',
    'http://127.0.0.1:18437/?query', 'http://127.0.0.1:18437/#fragment', 'http://127.0.0.1',
  ]) {
    assert.throws(() => externalAdapterConfig(['--expect-unix'], { ...env, CRONY_SERVER_HTTP: server }))
  }
  assert.equal(externalAdapterConfig(['--expect-unix'], {
    ...env, CRONY_SERVER_HTTP: 'http://127.0.0.1:8791', GITHUB_ACTIONS: 'true', CI: 'true',
  }).server, 'http://127.0.0.1:8791')
})

for (const os of ['linux', 'macos']) {
  test(os + ' requires explicit unavailability and native disabled-feature flags', () => {
    assert.equal(assertExternalRunner({ runners: [runner(os)] }, 'unix').os, os)
    assert.throws(() => assertExternalRunner({ runners: [runner(os, true)] }, 'unix'), /availability/)
    const unrelatedFailure = runner(os)
    unrelatedFailure.capabilities[0].detail = enabledSummary + '; model discovery failed: command not found'
    assert.throws(() => assertExternalRunner({ runners: [unrelatedFailure] }, 'unix'), /disabled execution features/)
  })
}

test('Windows unavailability is a failure, never an unsupported-platform success', () => {
  assert.equal(assertExternalRunner({ runners: [runner('windows')] }, 'windows').id, 'fixture-runner')
  assert.throws(() => assertExternalRunner({ runners: [runner('windows', false)] }, 'windows'), /availability/)
  assert.throws(() => assertExternalRunner({ runners: [runner('windows', false)] }, 'unix'), /runner OS/)
  assert.throws(() => assertExternalRunner({ runners: [runner('linux')] }, 'windows'), /runner OS/)
  assert.throws(() => assertExternalRunner({ runners: [runner('unknown')] }, 'unix'), /runner OS/)
})

test('missing, multiple, ambiguous and connection-bound capabilities cannot satisfy the legacy fixture', () => {
  assert.throws(() => assertExternalRunner({ runners: [] }, 'unix'), /exactly one/)
  assert.throws(() => assertExternalRunner({ runners: [runner('linux'), runner('linux')] }, 'unix'), /exactly one/)
  for (const capabilities of [
    [], [runner('linux').capabilities[0]],
    [...runner('linux').capabilities, runner('linux').capabilities[0]],
    runner('linux').capabilities.map(capability => ({ ...capability, workspace_connection_id: 'other-connection' })),
  ]) {
    assert.throws(() => assertExternalRunner({ runners: [{ ...runner('linux'), capabilities }] }, 'unix'), /capability/)
  }
})

function refusal() {
  const before = { snapshot: {
    tasks: [{ id: 'task-1', mission_id: 'mission-1', required_adapter: 'claude-code', status: 'ready' }],
    missions: [{ id: 'mission-1', status: 'ready' }], runs: [], events: [],
  } }
  return {
    adapter: 'claude-code', missionId: 'mission-1', before, after: structuredClone(before),
    launch: { status: 409, body: { error: 'mission dispatch incomplete (0 new runs dispatched): task task-1 requires adapter claude-code, but that adapter is unavailable' } },
  }
}

test('a platform refusal must prove the exact task, zero allocations and held work', () => {
  assertUnavailableLaunch(refusal())
  for (const corrupt of [
    input => { input.launch.status = 200 },
    input => { input.launch.status = 500 },
    input => { input.launch.body.error = 'another conflict' },
    input => { input.launch.body.error = input.launch.body.error.replace('task-1', 'different-task') },
    input => { input.after.snapshot.runs.push({ id: 'fallback', task_id: 'task-1' }) },
    input => { input.after.snapshot.events.push({ type: 'run.requested', correlation_id: 'mission-1' }) },
    input => { input.after.snapshot.events.push({ type: 'run.started', aggregate_id: 'task-1' }) },
    input => { input.after.snapshot.missions[0].status = 'running' },
    input => { input.after.snapshot.tasks[0].status = 'running' },
    input => { input.before.snapshot.tasks[0].required_adapter = 'fake-process' },
  ]) {
    const input = refusal()
    corrupt(input)
    assert.throws(() => assertUnavailableLaunch(input))
  }
})

function mockApi(os, { launchStatus, omitTermination = false, wrongArtifact = false, reconcilingPreviews = 0 } = {}) {
  const state = { runners: [runner(os)], snapshot: { missions: [], tasks: [], runs: [], events: [] } }
  const calls = []
  return {
    calls,
    wait: async () => {},
    fetchImpl: async (url, init) => {
      const pathname = new URL(url).pathname
      calls.push({ pathname, method: init.method ?? 'GET' })
      let status = 200
      let body
      if (pathname === '/api/demo/reset') {
        body = { corp_id: 'corp-1', alice_actor_id: 'alice' }
      } else if (pathname.endsWith('/snapshot')) {
        body = structuredClone(state)
      } else if (pathname.endsWith('/missions/preview')) {
        assert.equal(state.snapshot.missions.length, state.snapshot.runs.length,
          'Readiness must precede mission creation and must not retry a launch')
        if (reconcilingPreviews-- > 0) {
          status = 400
          body = { error: 'no connected runner can staff this source-bound mission' }
        } else body = { tasks: [] }
      } else if (pathname.endsWith('/missions')) {
        const request = JSON.parse(init.body)
        const number = state.snapshot.missions.length + 1
        const missionId = 'mission-' + number
        state.snapshot.missions.push({ id: missionId, status: 'ready' })
        state.snapshot.tasks.push({ id: 'task-' + number, mission_id: missionId, required_adapter: request.preferred_adapter, status: 'ready' })
        status = 201
        body = { mission_id: missionId }
      } else if (pathname.endsWith('/launch')) {
        const missionId = pathname.split('/').at(-2)
        const task = state.snapshot.tasks.find(task => task.mission_id === missionId)
        status = launchStatus ?? (os === 'windows' ? 200 : 409)
        if (status === 200) {
          const run = {
            id: 'run-' + task.id, task_id: task.id, runner_id: 'fixture-runner',
            provider_session_id: 'session-' + task.id, status: 'completed', workspace_disposition: 'removed',
            artifact_sha256: 'digest-' + task.id, input_tokens: 100, output_tokens: 40, provider: task.required_adapter,
          }
          state.snapshot.runs.push(run)
          if (!omitTermination) state.snapshot.events.push({
            aggregate_id: run.id, type: 'run.session_terminated', seq: 1, payload: { provider_process_alive: false },
          })
          state.snapshot.events.push({ aggregate_id: run.id, type: 'run.completed', seq: 2 })
          body = { run_id: run.id }
        } else {
          body = { error: 'mission dispatch incomplete (0 new runs dispatched): task ' + task.id +
            ' requires adapter ' + task.required_adapter + ', but that adapter is unavailable' }
        }
      } else throw new Error('Unexpected fixture request: ' + pathname)
      return { status, json: async () => body }
    },
    downloadArtifact: async (_server, _demo, run) => Buffer.from(JSON.stringify({
      provider: wrongArtifact ? 'other' : run.provider, exit_success: true,
    })),
  }
}

test('the Unix suite actually attempts both adapters and reports rejection, not lifecycle success', async () => {
  const api = mockApi('linux')
  const report = await runExternalAdapterContract({ server: env.CRONY_SERVER_HTTP, expectedPlatform: 'unix' }, api)
  assert.equal(api.calls.filter(call => call.pathname.endsWith('/launch')).length, 2)
  assert.equal(report.coverage, 'unsupported_admission')
  assert.equal(report.common_sample, false)
  assert.deepEqual(report.providers.map(result => result.adapter), providers)
  assert.ok(report.providers.every(result => result.runs_created === 0 && result.launch_status === 409))
})

test('unrelated HTTP errors cannot be recorded as a successful Unix refusal', async () => {
  await assert.rejects(runExternalAdapterContract(
    { server: env.CRONY_SERVER_HTTP, expectedPlatform: 'unix' }, mockApi('linux', { launchStatus: 500 }),
  ), /Unsupported execution/)
})

test('the Windows suite retains session, usage, artifact and termination evidence for both providers', async () => {
  const report = await runExternalAdapterContract(
    { server: env.CRONY_SERVER_HTTP, expectedPlatform: 'windows' }, mockApi('windows'),
  )
  assert.equal(report.coverage, 'fixture_lifecycle')
  assert.equal(report.common_sample, true)
  assert.equal(report.real_provider_inference, false)
  assert.deepEqual(report.providers.map(result => result.adapter), providers)
  assert.ok(report.providers.every(result => result.outcome === 'completed' && result.provider_process_alive === false))
})

test('Windows evidence rejects missing termination and a mismatched provider artifact', async () => {
  for (const failure of [{ omitTermination: true }, { wrongArtifact: true }]) {
    await assert.rejects(runExternalAdapterContract(
      { server: env.CRONY_SERVER_HTTP, expectedPlatform: 'windows' }, mockApi('windows', failure),
    ))
  }
})

test('Windows readiness retries only native read-only previews, with one launch per provider', async () => {
  const api = mockApi('windows', { reconcilingPreviews: 2 })
  const report = await runExternalAdapterContract(
    { server: env.CRONY_SERVER_HTTP, expectedPlatform: 'windows' }, api,
  )
  assert.deepEqual(report.providers.map(result => result.readiness_previews), [3, 1])
  assert.equal(api.calls.filter(call => call.pathname.endsWith('/launch')).length, 2)
})

test('native readiness has a bound and cannot launch while reconciliation remains incomplete', async () => {
  const api = mockApi('windows', { reconcilingPreviews: 100 })
  let clock = 0
  await assert.rejects(runExternalAdapterContract(
    { server: env.CRONY_SERVER_HTTP, expectedPlatform: 'windows' },
    { ...api, now: () => { clock += 20_000; return clock } },
  ), /Timed out waiting for read-only native dispatch readiness/)
  assert.equal(api.calls.some(call => call.pathname.endsWith('/launch')), false)
})

test('CI wires a Unix refusal and a separate Windows lifecycle fixture without uploading private state', () => {
  const workflow = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8').replaceAll('\r\n', '\n')
  const unix = workflow.split('\n  integration:')[1].split('\n  external-adapters-windows:')[0]
  const windows = workflow.split('\n  external-adapters-windows:')[1].split('\n  runner-platforms:')[0]
  assert.match(unix, /runs-on: ubuntu-latest/)
  assert.match(unix, /e2e_external_adapters\.mjs --expect-unix --dry-run/)
  assert.match(unix, /e2e_external_adapters\.mjs --expect-unix\n/)
  assert.match(windows, /runs-on: windows-latest/)
  assert.match(windows, /ci_external_adapters_windows\.ps1.*-DryRun/)
  assert.match(windows, /ci_external_adapters_windows\.ps1.*-Execute/)
  assert.match(windows, /path:.*ecorp-external-adapters-ci\/evidence\//)
  assert.doesNotMatch(windows, /continue-on-error|credential\.json|pg-data\/|runner-workspaces\//)
})
