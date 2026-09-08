import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

// node --test runs this file in its own child scope. Never read the ambient URL,
// spread process.env, or pass provider credentials into the PowerShell fixtures.
for (const name of Object.keys(process.env)) {
  if (name.toUpperCase() === 'DATABASE_URL') delete process.env[name]
}
const environment = {}
for (const name of [
  'SystemRoot', 'WINDIR', 'ComSpec', 'PATH', 'PATHEXT', 'TEMP', 'TMP',
  'PSModulePath', 'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432',
]) {
  if (process.env[name] !== undefined) environment[name] = process.env[name]
}

const directory = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(directory, 'local_stack_lifecycle.test.ps1')
const resultPrefix = 'ECORP_LOCAL_STACK_TEST_RESULT='

async function runSuite(t, suite) {
  const result = spawnSync('pwsh.exe', [
    '-NoLogo', '-NoProfile', '-NonInteractive', '-File', script,
    '-Suite', suite, '-NodePath', process.execPath,
  ], {
    cwd: directory,
    env: environment,
    encoding: 'utf8',
    windowsHide: true,
    timeout: 120_000,
    maxBuffer: 2 * 1024 * 1024,
  })
  assert.ifError(result.error)
  const reports = (result.stdout ?? '').split(/\r?\n/u)
    .filter((line) => line.startsWith(resultPrefix))
  // Do not echo arbitrary module output (even synthetic canaries) on failure.
  assert.equal(reports.length, 1,
    `Expected one structured ${suite} report; PowerShell exit=${result.status}, signal=${result.signal}`)
  const report = JSON.parse(reports[0].slice(resultPrefix.length))
  assert.equal(report.suite, suite)
  assert.equal(report.scope, 'synthetic-only; not native startup acceptance')
  assert.ok(Array.isArray(report.cases) && report.cases.length > 0)
  assert.equal(new Set(report.cases.map((entry) => entry.name)).size, report.cases.length)
  for (const entry of report.cases) {
    await t.test(entry.name, () => {
      assert.equal(entry.passed, true, entry.error || entry.name)
    })
  }
  if (suite === 'Module') {
    assert.equal(report.cleanup.remaining_processes, 0, 'Synthetic fixture processes must be reaped')
    assert.equal(report.cleanup.temp_removed, true, 'Only the task-created temporary tree must be removed')
    if (report.cases.every((entry) => entry.passed)) {
      assert.ok(report.cleanup.created_processes > 0, 'The native fixtures must actually execute')
    }
    t.diagnostic(`PowerShell ${report.powershell}; ${report.cleanup.created_processes} synthetic processes reaped; no fixture tree remains`)
  }
  assert.equal(result.status, report.cases.every((entry) => entry.passed) ? 0 : 1,
    'PowerShell exit status must agree with the reported cases')
}

// Module fixtures may execute the AST-extracted Launch function against owned
// children and a locked temporary state file; neither starter entrypoint runs.
test('local stack owned-process and state regression fixtures', {
  skip: process.platform !== 'win32' ? 'Requires Windows and PowerShell 7.4+' : false,
}, async (t) => runSuite(t, 'Module'))

// While implementation is in progress, run only the module test with
// --test-name-pattern=owned-process. These checks parse, never execute, starters.
test('local stack starter source guards', {
  skip: process.platform !== 'win32' ? 'Requires the PowerShell AST parser' : false,
}, async (t) => runSuite(t, 'Source'))
