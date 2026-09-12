import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { downloadVerifiedArtifact } from './artifact_client.mjs'

const root = path.resolve(import.meta.dirname, '..')
const adapters = ['claude-code', 'opencode']
const unsupportedReason = 'non-escapable process boundary'

export function externalAdapterConfig(args, env) {
  assert.ok(args.every(arg => ['--expect-windows', '--expect-unix', '--dry-run'].includes(arg)),
    'Unknown external-adapter fixture option')
  const modes = args.filter(arg => arg !== '--dry-run')
  assert.equal(modes.length, 1, 'Select exactly one of --expect-windows or --expect-unix')
  assert.equal(env.CRONY_EXTERNAL_ADAPTER_TEST, '1',
    'Set CRONY_EXTERNAL_ADAPTER_TEST=1 only for an owned disposable fixture')
  assert.ok(env.CRONY_SERVER_HTTP, 'An explicit owned CRONY_SERVER_HTTP is required')
  const endpoint = new URL(env.CRONY_SERVER_HTTP)
  assert.ok(endpoint.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) &&
    endpoint.port && endpoint.pathname === '/' && !endpoint.search && !endpoint.hash &&
    !endpoint.username && !endpoint.password, 'The fixture requires an explicit loopback HTTP origin')
  const manualPorts = new Set(['8791', '8793', '5187', '5291', '15191', '15193'])
  assert.ok(!manualPorts.has(endpoint.port) || (env.GITHUB_ACTIONS === 'true' && env.CI === 'true'),
    'Refusing a manual-stack port outside the disposable GitHub Actions job')
  return {
    server: endpoint.origin,
    expectedPlatform: modes[0] === '--expect-windows' ? 'windows' : 'unix',
    dryRun: args.includes('--dry-run'),
    output: path.resolve(env.CRONY_EXTERNAL_ADAPTER_OUTPUT ?? path.join(root, 'output', 'e2e-external-adapters.json')),
  }
}

export function assertExternalRunner(state, expectedPlatform) {
  assert.ok(['windows', 'unix'].includes(expectedPlatform), 'Unknown platform expectation')
  const runners = state.runners.filter(runner => runner.connected)
  assert.equal(runners.length, 1, 'This fixture requires exactly one connected runner')
  const runner = runners[0]
  // The runner's OS is authoritative, not the HTTP client's process.platform.
  assert.ok(expectedPlatform === 'windows' ? runner.os === 'windows' : ['linux', 'macos'].includes(runner.os),
    `Expected ${expectedPlatform} execution, observed runner OS ${runner.os}`)
  for (const adapter of adapters) {
    const matches = runner.capabilities.filter(capability =>
      capability.name === adapter && capability.workspace_connection_id == null)
    assert.equal(matches.length, 1, `Missing or ambiguous legacy ${adapter} capability`)
    assert.equal(matches[0].available, expectedPlatform === 'windows',
      `${adapter} availability contradicts the expected platform contract`)
    if (expectedPlatform === 'unix') {
      assert.ok(matches[0].detail?.includes(unsupportedReason),
        `${adapter} must report the process-containment restriction, not an unrelated probe failure`)
    }
  }
  return runner
}

export function assertUnavailableLaunch({ adapter, missionId, before, after, launch }) {
  const tasks = before.snapshot.tasks.filter(task => task.mission_id === missionId)
  assert.equal(tasks.length, 1, 'The common sample must contain one task')
  assert.equal(tasks[0].required_adapter, adapter)
  assert.equal(launch.status, 409, 'Unsupported execution must be rejected, not skipped')
  assert.ok(launch.body.error?.includes('mission dispatch incomplete (0 new runs dispatched)'))
  assert.ok(launch.body.error?.includes(`task ${tasks[0].id} requires adapter ${adapter}, but that adapter is unavailable`))
  assert.deepEqual(after.snapshot.runs.map(run => run.id).sort(), before.snapshot.runs.map(run => run.id).sort(),
    'A rejected launch must not allocate a run or fall back to another adapter')
  assert.equal(after.snapshot.runs.some(run => run.task_id === tasks[0].id), false)
  const mission = after.snapshot.missions.find(candidate => candidate.id === missionId)
  assert.equal(mission?.status, 'ready', 'Rejected work must remain held')
  assert.equal(after.snapshot.tasks.find(task => task.id === tasks[0].id)?.status, tasks[0].status)
}

export async function runExternalAdapterContract({ server, expectedPlatform }, {
  fetchImpl = fetch,
  downloadArtifact = downloadVerifiedArtifact,
  wait = ms => new Promise(resolve => setTimeout(resolve, ms)),
  now = Date.now,
} = {}) {
  async function request(route, body) {
    const response = await fetchImpl(`${server}${route}`, {
      redirect: 'error',
      signal: AbortSignal.timeout(10_000),
      ...(body === undefined ? {} : {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
      }),
    })
    return { status: response.status, body: await response.json() }
  }
  async function ok(route, body) {
    const result = await request(route, body)
    assert.ok(result.status >= 200 && result.status < 300, `${route}: ${JSON.stringify(result)}`)
    return result.body
  }

  const demo = await ok('/api/demo/reset', {})
  const prefix = `/api/corps/${demo.corp_id}`
  const snapshot = () => ok(`${prefix}/snapshot?actor_id=${demo.alice_actor_id}`)
  const initial = await snapshot()
  const runner = assertExternalRunner(initial, expectedPlatform)
  assert.equal(initial.snapshot.runs.length, 0, 'Reset fixture unexpectedly contains prior runs')
  const results = []
  for (const adapter of adapters) {
    const missionRequest = {
      requested_by: demo.alice_actor_id,
      preferred_adapter: adapter,
      title: `Common provider parity sample for ${adapter}`,
    }
    let readinessPreviews = 0
    if (expectedPlatform === 'windows') {
      const workspace = runner.capabilities.find(capability => capability.name === 'workspace-isolation' &&
        capability.workspace_connection_id == null && capability.available)
      assert.ok(workspace?.source_repository && workspace.source_base_ref && workspace.source_base_commit,
        'The Windows fixture needs a checked source identity for native readiness preview')
      const source = { repository: workspace.source_repository, base_ref: workspace.source_base_ref,
        base_commit: workspace.source_base_commit }
      const deadline = now() + 60_000
      let ready = false
      while (now() < deadline) {
        // Registration is visible before native reconnect reconciliation enables dispatch.
        // A source-bound mission preview checks that barrier without creating workers,
        // missions or runs. Never retry an effectful launch to hide startup races.
        const preview = await request(`${prefix}/missions/preview`, { ...missionRequest, source })
        readinessPreviews++
        if (preview.status === 200) { ready = true; break }
        assert.equal(preview.status, 400, JSON.stringify(preview))
        assert.match(preview.body.error ?? '', /no connected runner can staff|no matching runner was selectable/u)
        assert.equal(assertExternalRunner(await snapshot(), expectedPlatform).id, runner.id)
        await wait(100)
      }
      assert.ok(ready, 'Timed out waiting for read-only native dispatch readiness')
    }
    const mission = await ok(`${prefix}/missions`, missionRequest)
    const before = await snapshot()
    const launch = await request(`${prefix}/missions/${mission.mission_id}/launch`,
      { requested_by: demo.alice_actor_id })
    if (expectedPlatform === 'unix') {
      const after = await snapshot()
      assertExternalRunner(after, expectedPlatform)
      assertUnavailableLaunch({ adapter, missionId: mission.mission_id, before, after, launch })
      results.push({ adapter, outcome: 'unsupported_rejected', mission_id: mission.mission_id,
        launch_status: launch.status, runs_created: 0 })
      continue
    }

    assert.equal(launch.status, 200, JSON.stringify(launch))
    const deadline = Date.now() + 30_000
    let result
    while (Date.now() < deadline) {
      const state = await snapshot()
      const run = state.snapshot.runs.find(candidate => candidate.id === launch.body.run_id)
      if (run?.status === 'completed' && ['preserved', 'removed'].includes(run.workspace_disposition)) {
        const evidence = JSON.parse(await downloadArtifact(server, demo, run))
        assert.equal(evidence.provider, adapter)
        assert.equal(evidence.exit_success, true)
        assert.ok(run.provider_session_id)
        assert.equal(run.runner_id, runner.id)
        assert.ok(run.input_tokens > 0 && run.output_tokens > 0)
        const events = state.snapshot.events.filter(event => event.aggregate_id === run.id)
        const terminated = events.find(event => event.type === 'run.session_terminated')
        const completed = events.find(event => event.type === 'run.completed')
        assert.equal(terminated?.payload.provider_process_alive, false)
        assert.ok(completed && terminated.seq < completed.seq, 'Provider must terminate before accepted completion')
        result = {
          adapter, outcome: 'completed', run_id: run.id, provider_session_id: run.provider_session_id,
          artifact_sha256: run.artifact_sha256, input_tokens: run.input_tokens, output_tokens: run.output_tokens,
          provider_process_alive: false, terminal_event_seq: terminated.seq, completion_event_seq: completed.seq,
          readiness_previews: readinessPreviews,
        }
        break
      }
      if (run && ['failed', 'cancelled', 'lost'].includes(run.status)) {
        throw new Error(`${adapter} run ended as ${run.status}: ${run.summary}`)
      }
      await wait(100)
    }
    assert.ok(result, `Timed out waiting for ${adapter}`)
    results.push(result)
  }
  if (expectedPlatform === 'windows') {
    assert.equal(new Set(results.map(result => result.artifact_sha256)).size, 2)
  }
  return {
    schema_version: 2, checked_at: new Date().toISOString(), expected_platform: expectedPlatform,
    runner_os: runner.os, runner_id: runner.id,
    coverage: expectedPlatform === 'windows' ? 'fixture_lifecycle' : 'unsupported_admission',
    common_sample: expectedPlatform === 'windows', real_provider_inference: false, providers: results,
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const config = externalAdapterConfig(process.argv.slice(2), process.env)
  if (config.dryRun) {
    console.log(JSON.stringify({ ...config, proposed: [
      'reset only the explicitly owned demo fixture',
      'verify the connected runner OS and both external adapter capabilities',
      config.expectedPlatform === 'windows' ? 'execute both synthetic provider lifecycles and verify their artifacts' :
        'attempt both unavailable adapters and require HTTP 409, zero runs and held missions',
    ], services_started: false, database_writes: false }, null, 2))
  } else {
    const report = await runExternalAdapterContract(config)
    await mkdir(path.dirname(config.output), { recursive: true })
    await writeFile(config.output, `${JSON.stringify(report, null, 2)}\n`)
    console.log(JSON.stringify(report, null, 2))
  }
}
