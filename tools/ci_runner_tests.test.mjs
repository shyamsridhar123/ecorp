import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'

test('Windows runner tests are serialized without filtering tests or changing Unix scheduling', () => {
  const workflow = readFileSync(path.join(import.meta.dirname, '..', '.github', 'workflows', 'ci.yml'), 'utf8')
    .replace(/\r\n/gu, '\n')
  const matrix = workflow.split('  runner-platforms:')[1].split('  desktop-windows:')[0]
  assert.match(matrix, /os: \[ubuntu-latest, windows-latest, macos-latest\]/u)
  assert.match(matrix, /fail-fast: false/u)
  assert.match(matrix, /name: Run Windows runner tests serially\n\s+if: runner.os == 'Windows'\n\s+run: cargo test -p crony-runner -- --test-threads=1\n/u)
  assert.match(matrix, /name: Run Unix runner tests\n\s+if: runner.os != 'Windows'\n\s+run: cargo test -p crony-runner\n/u)
  assert.match(matrix, /run: node tools\/platform_runner_contract\.mjs/u)
  assert.doesNotMatch(matrix, /--skip|--ignored|--exclude|continue-on-error|RUST_TEST_THREADS/u)
  assert.equal((matrix.match(/--test-threads=1/gu) ?? []).length, 1)
})
