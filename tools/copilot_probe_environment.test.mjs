import assert from 'node:assert/strict'
import test from 'node:test'
import { assertCanaryEnvironmentEvidence } from './copilot_probe_environment.mjs'

const options = {
  probeId: 'deterministic-environment-test',
  runnerObserverPid: 101,
  minimumProviderProcesses: 2,
}

function observed() {
  return [
    {
      kind: 'runner_environment', probe_id: options.probeId, pid: 101,
      canary_present: true, github_token_present: true, github_token_is_canary: true,
    },
    {
      kind: 'runner_spawn', probe_id: options.probeId, pid: 101, child_pid: 202,
    },
    ...[303, 404].map((pid) => ({
      kind: 'copilot_environment', probe_id: options.probeId, pid, parent_pid: 202,
      canary_present: false, github_token_present: false, github_token_is_canary: false,
      auto_update_disabled_by_flag: true,
    })),
  ]
}

test('accepts an exact seed and credential-free children of the owned runner', () => {
  const records = observed()
  assert.deepEqual(assertCanaryEnvironmentEvidence(records, options), records.slice(2))
})

test('a marker elsewhere or an arbitrary GITHUB_TOKEN is not a proven seed', () => {
  for (const change of [
    { github_token_present: false, github_token_is_canary: false },
    { github_token_present: true, github_token_is_canary: false },
    { github_token_present: true, github_token_is_canary: undefined },
  ]) {
    const records = observed()
    Object.assign(records[0], change)
    assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /canary precondition/u)
  }
})

test('absent, ambiguous or foreign runner seed records cannot establish the precondition', () => {
  const valid = observed()
  for (const records of [
    valid.slice(1),
    [{ ...valid[0], probe_id: 'another-probe' }, ...valid.slice(1)],
    [{ ...valid[0], pid: 999 }, ...valid.slice(1)],
    [valid[0], valid[0], ...valid.slice(1)],
  ]) {
    assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /seed/u)
  }
})

test('runner seed booleans must be explicit, not truthy or absent', () => {
  for (const key of ['canary_present', 'github_token_present', 'github_token_is_canary']) {
    for (const value of [false, undefined, null, 'true', 1]) {
      const records = observed()
      records[0][key] = value
      assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /canary precondition/u)
    }
  }
})

test('missing spawn and undefined parent IDs cannot accidentally correlate', () => {
  const records = observed().filter((entry) => entry.kind !== 'runner_spawn')
  for (const entry of records.filter((entry) => entry.kind === 'copilot_environment')) {
    delete entry.parent_pid
  }
  assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /runner spawn/u)
})

test('runner spawn must be unique, current and tied to the owned observer', () => {
  for (const change of [
    { probe_id: 'another-probe' }, { pid: 999 }, { pid: undefined },
    { child_pid: undefined }, { child_pid: 0 }, { child_pid: '202' }, { child_pid: 101 },
  ]) {
    const records = observed()
    Object.assign(records[1], change)
    assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /runner spawn/u)
  }
  const records = observed()
  records.push({ ...records[1] })
  assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /runner spawn/u)
})

test('a seed observed only after runner spawn cannot prove the launch precondition', () => {
  const records = observed()
  ;[records[0], records[1]] = [records[1], records[0]]
  assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /before spawning/u)
})

test('missing or foreign provider observations cannot satisfy the required count', () => {
  for (const records of [
    observed().slice(0, 2),
    observed().slice(0, 3),
    observed().map((entry) => entry.kind === 'copilot_environment'
      ? { ...entry, probe_id: 'another-probe' } : entry),
  ]) {
    assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /every required provider/u)
  }
})

test('a wrong-parent provider is rejected rather than silently filtered out', () => {
  const records = observed()
  records.push({ ...records[2], pid: 505, parent_pid: 999 })
  assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /observed runner child/u)
})

test('provider identity and absence booleans must be explicit and valid', () => {
  for (const change of [
    { pid: undefined }, { pid: 0 }, { pid: '303' }, { pid: 101 }, { pid: 202 },
    { parent_pid: undefined }, { parent_pid: '202' },
    { canary_present: true }, { canary_present: undefined }, { canary_present: 'false' },
    { github_token_present: true }, { github_token_present: undefined },
    { github_token_is_canary: true }, { github_token_is_canary: undefined },
  ]) {
    const records = observed()
    Object.assign(records[2], change)
    assert.throws(() => assertCanaryEnvironmentEvidence(records, options), /provider/u)
  }
})

test('missing ownership and zero or malformed minimums never allow a vacuous pass', () => {
  for (const change of [
    { probeId: '' }, { runnerObserverPid: undefined }, { runnerObserverPid: '101' },
    { minimumProviderProcesses: 0 }, { minimumProviderProcesses: -1 },
    { minimumProviderProcesses: '2' }, { minimumProviderProcesses: 1.5 },
  ]) {
    assert.throws(() => assertCanaryEnvironmentEvidence(observed(), { ...options, ...change }))
  }
  assert.throws(() => assertCanaryEnvironmentEvidence(null, options))
  assert.throws(() => assertCanaryEnvironmentEvidence([...observed(), null], options))
})
