import assert from 'node:assert/strict'

// Public synthetic marker, never an authentication credential.
export const COPILOT_CREDENTIAL_CANARY = 'ECORP_CREDENTIAL_CANARY_MUST_NOT_REACH_COPILOT'

const validPid = (value) => Number.isSafeInteger(value) && value > 0

export function assertCanaryEnvironmentEvidence(observations, {
  probeId, runnerObserverPid, minimumProviderProcesses,
}) {
  assert.ok(Array.isArray(observations), 'environment observations are required')
  assert.ok(typeof probeId === 'string' && probeId.length > 0, 'probe identity is required')
  assert.ok(validPid(runnerObserverPid), 'the owned runner observer PID is required')
  assert.ok(Number.isSafeInteger(minimumProviderProcesses) && minimumProviderProcesses > 0,
    'a positive provider observation minimum is required')
  assert.ok(observations.every((entry) => entry && typeof entry === 'object'),
    'environment observation records must be objects')

  const current = observations.filter((entry) => entry.probe_id === probeId)
  const seeds = current.filter((entry) => entry.kind === 'runner_environment')
  assert.equal(seeds.length, 1, 'exactly one runner seed observation is required')
  const [seed] = seeds
  assert.ok(seed.pid === runnerObserverPid, 'seed must belong to the owned runner observer')
  // A marker elsewhere in the environment, or a failed observer's partial
  // record, cannot prove that GITHUB_TOKEN carried the exact synthetic seed.
  assert.ok(seed.canary_present === true && seed.github_token_present === true &&
    seed.github_token_is_canary === true, 'runner GITHUB_TOKEN canary precondition is unproven')

  const spawns = current.filter((entry) => entry.kind === 'runner_spawn')
  assert.equal(spawns.length, 1, 'exactly one runner spawn observation is required')
  const [runnerSpawn] = spawns
  assert.ok(runnerSpawn.pid === runnerObserverPid &&
    validPid(runnerSpawn.child_pid) && runnerSpawn.child_pid !== runnerObserverPid,
  'runner spawn must identify the owned observer and its actual child')
  assert.ok(current.indexOf(seed) < current.indexOf(runnerSpawn),
    'runner seed must be observed before spawning the runner')

  const providers = current.filter((entry) => entry.kind === 'copilot_environment')
  assert.ok(providers.length >= minimumProviderProcesses,
    'observe catalog plus every required provider process environment')
  for (const entry of providers) {
    assert.ok(validPid(entry.pid) && entry.pid !== runnerObserverPid &&
      entry.pid !== runnerSpawn.child_pid && entry.parent_pid === runnerSpawn.child_pid,
    'provider environment must belong to the observed runner child')
    assert.ok(entry.canary_present === false && entry.github_token_present === false &&
      entry.github_token_is_canary === false,
    'provider credential removal must be explicitly observed')
  }
  return providers
}
