import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { downloadVerifiedArtifact } from './artifact_client.mjs'

const server = process.env.CRONY_SERVER_HTTP ?? 'http://127.0.0.1:8791'
const root = path.resolve(import.meta.dirname, '..')

async function request(url, init) {
  const response = await fetch(`${server}${url}`, init)
  const body = await response.json()
  if (!response.ok) {
    throw new Error(`${init?.method ?? 'GET'} ${url} failed: ${JSON.stringify(body)}`)
  }
  return body
}

function post(url, body) {
  return request(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

async function snapshot(demo) {
  return request(
    `/api/corps/${demo.corp_id}/snapshot?actor_id=${demo.alice_actor_id}`,
  )
}

async function waitForRun(demo, runId, predicate, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await snapshot(demo)
    const run = state.snapshot.runs.find((candidate) => candidate.id === runId)
    if (run && predicate(run)) return { run, state }
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error(`timed out waiting for run ${runId}`)
}

function isSettled(run) {
  return (
    ['completed', 'failed', 'cancelled', 'lost'].includes(run.status) &&
    ['preserved', 'removed'].includes(run.workspace_disposition)
  )
}

async function assertArtifact(demo, run) {
  await downloadVerifiedArtifact(server, demo, run)
  return run.artifact_uri
}

async function createCodexMission(demo, title) {
  const mission = await post(`/api/corps/${demo.corp_id}/missions`, {
    requested_by: demo.alice_actor_id,
    preferred_adapter: 'codex',
    title,
  })
  const launch = await post(
    `/api/corps/${demo.corp_id}/missions/${mission.mission_id}/launch`,
    { requested_by: demo.alice_actor_id },
  )
  return { mission, launch }
}

async function startSteerResumeScenario() {
  const demo = await post('/api/demo/reset', {})
  const lease = await post(
    `/api/corps/${demo.corp_id}/agents/${demo.codex_agent_id}/lease`,
    { actor_id: demo.alice_actor_id },
  )
  assert.equal(lease.acquired, true)

  const { mission, launch } = await createCodexMission(
    demo,
    '[slow] create a base file and apply live direction',
  )
  const started = await waitForRun(
    demo,
    launch.run_id,
    (run) => Boolean(run.provider_session_id),
  )
  const message = await post(
    `/api/corps/${demo.corp_id}/agents/${demo.codex_agent_id}/messages`,
    {
      actor_id: demo.alice_actor_id,
      lease_token: lease.token,
      text: 'create the steered file',
    },
  )
  assert.equal(message.delivery, 'immediate')

  const completed = await waitForRun(
    demo,
    launch.run_id,
    isSettled,
  )
  assert.equal(completed.run.status, 'completed')
  assert.ok(completed.run.input_tokens > 0)
  assert.ok(completed.run.output_tokens > 0)
  assert.ok(completed.run.workspace_path, 'run omitted worktree path')
  const workspace = completed.run.workspace_path
  assert.equal(completed.run.workspace_run_id, launch.run_id)
  assert.equal(completed.run.workspace_disposition, 'preserved')
  assert.ok(completed.run.workspace_branch?.startsWith('crony/task-'))
  assert.equal(await readFile(path.join(workspace, 'base.txt'), 'utf8'), 'base\n')
  assert.equal(
    await readFile(path.join(workspace, 'steered.txt'), 'utf8'),
    'create the steered file\n',
  )
  const startArtifact = await assertArtifact(demo, completed.run)

  const resume = await post(
    `/api/corps/${demo.corp_id}/runs/${launch.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'resume the prior task',
    },
  )
  const resumed = await waitForRun(
    demo,
    resume.run_id,
    isSettled,
  )
  assert.equal(resumed.run.status, 'completed')
  assert.equal(resumed.run.provider_session_id, started.run.provider_session_id)
  assert.equal(resumed.run.resumed_from_run_id, launch.run_id)
  assert.equal(resumed.run.workspace_path, workspace)
  assert.equal(resumed.run.workspace_branch, completed.run.workspace_branch)
  assert.equal(resumed.run.workspace_run_id, launch.run_id)
  assert.equal(resumed.run.workspace_disposition, 'preserved')
  assert.ok(resumed.run.input_tokens > 0)
  assert.equal(
    await readFile(path.join(workspace, 'resumed.txt'), 'utf8'),
    'resumed\n',
  )
  const resumeArtifact = await assertArtifact(demo, resumed.run)

  const secondResume = await post(
    `/api/corps/${demo.corp_id}/runs/${resume.run_id}/resume`,
    {
      requested_by: demo.alice_actor_id,
      prompt: 'resume the task lineage again',
    },
  )
  const resumedAgain = await waitForRun(
    demo,
    secondResume.run_id,
    isSettled,
  )
  assert.equal(resumedAgain.run.status, 'completed')
  assert.equal(resumedAgain.run.workspace_run_id, launch.run_id)
  assert.equal(resumedAgain.run.workspace_path, workspace)
  assert.equal(resumedAgain.run.workspace_branch, completed.run.workspace_branch)
  assert.equal(resumedAgain.run.resumed_from_run_id, resume.run_id)
  const secondResumeArtifact = await assertArtifact(demo, resumedAgain.run)

  return {
    corp_id: demo.corp_id,
    mission_id: mission.mission_id,
    run_id: launch.run_id,
    resume_run_id: resume.run_id,
    second_resume_run_id: secondResume.run_id,
    provider_session_id: resumed.run.provider_session_id,
    steer_delivery: message.delivery,
    start_status: completed.run.status,
    resume_status: resumed.run.status,
    second_resume_status: resumedAgain.run.status,
    start_usage: {
      input_tokens: completed.run.input_tokens,
      output_tokens: completed.run.output_tokens,
    },
    resume_usage: {
      input_tokens: resumed.run.input_tokens,
      output_tokens: resumed.run.output_tokens,
    },
    start_artifact: startArtifact,
    resume_artifact: resumeArtifact,
    second_resume_artifact: secondResumeArtifact,
  }
}

async function interruptScenario() {
  const demo = await post('/api/demo/reset', {})
  const lease = await post(
    `/api/corps/${demo.corp_id}/agents/${demo.codex_agent_id}/lease`,
    { actor_id: demo.alice_actor_id },
  )
  assert.equal(lease.acquired, true)
  const { launch } = await createCodexMission(
    demo,
    '[slow] create a base file before interruption',
  )
  const started = await waitForRun(
    demo,
    launch.run_id,
    (run) => Boolean(run.provider_session_id),
  )
  const interrupt = await post(
    `/api/corps/${demo.corp_id}/agents/${demo.codex_agent_id}/interrupt`,
    {
      actor_id: demo.alice_actor_id,
      lease_token: lease.token,
      reason: 'deterministic Codex interrupt test',
    },
  )
  assert.equal(interrupt.requested, true)
  const terminal = await waitForRun(
    demo,
    launch.run_id,
    isSettled,
  )
  assert.equal(terminal.run.status, 'cancelled')
  const artifact = await assertArtifact(demo, terminal.run)
  return {
    run_id: launch.run_id,
    provider_session_id: started.run.provider_session_id,
    status: terminal.run.status,
    artifact,
  }
}

async function stopScenario() {
  const demo = await post('/api/demo/reset', {})
  const { launch } = await createCodexMission(
    demo,
    '[slow] create a base file before emergency stop',
  )
  await waitForRun(
    demo,
    launch.run_id,
    (run) => Boolean(run.provider_session_id),
  )
  const stop = await post(
    `/api/corps/${demo.corp_id}/agents/${demo.codex_agent_id}/emergency-stop`,
    {
      actor_id: demo.alice_actor_id,
      reason: 'deterministic Codex emergency-stop test',
    },
  )
  assert.equal(stop.requested, true)
  const terminal = await waitForRun(
    demo,
    launch.run_id,
    isSettled,
  )
  assert.equal(terminal.run.status, 'cancelled')
  const artifact = await assertArtifact(demo, terminal.run)
  return {
    run_id: launch.run_id,
    status: terminal.run.status,
    artifact,
  }
}

const report = {
  checked_at: new Date().toISOString(),
  start_steer_resume: await startSteerResumeScenario(),
  interrupt: await interruptScenario(),
  stop: await stopScenario(),
}

await writeFile(
  path.join(root, 'output', 'e2e-codex.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)
console.log(JSON.stringify(report, null, 2))
