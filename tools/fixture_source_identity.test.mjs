import assert from 'node:assert/strict'
import test from 'node:test'
import {
  parseFixtureSourceIdentity,
  readFixtureSourceIdentity,
} from './fixture_source_identity.mjs'

const transferred = {
  owner: 'all-the-vibes',
  name: 'ecorp',
  repository: 'all-the-vibes/ecorp',
  url: 'https://github.com/all-the-vibes/ecorp',
}

for (const [name, origin] of [
  ['HTTPS clone URL', 'https://github.com/All-The-Vibes/ecorp.git'],
  ['HTTPS repository URL', 'https://github.com/All-The-Vibes/ecorp'],
  ['HTTPS default port', 'https://github.com:443/All-The-Vibes/ecorp.git'],
  ['SSH SCP URL', 'git@github.com:All-The-Vibes/ecorp.git'],
  ['SSH URI', 'ssh://git@github.com/All-The-Vibes/ecorp.git'],
  ['SSH default port', 'ssh://git@github.com:22/All-The-Vibes/ecorp.git'],
  ['SSH alternate port', 'ssh://git@github.com:443/All-The-Vibes/ecorp.git'],
  ['case normalization', 'HTTPS://GITHUB.COM/ALL-THE-VIBES/ECorp.git'],
  ['trailing slash', 'https://github.com/All-The-Vibes/ecorp.git/'],
  ['Git output newline', 'https://github.com/All-The-Vibes/ecorp.git\r\n'],
]) {
  test(`parses ${name} without resolving redirects`, () => {
    assert.deepEqual(parseFixtureSourceIdentity(origin), transferred)
  })
}

test('keeps old, transferred and deliberately wrong repositories distinct', () => {
  const legacy = parseFixtureSourceIdentity(
    'https://github.com/shyamsridhar123/ecorp.git',
  )
  const current = parseFixtureSourceIdentity(
    'https://github.com/All-The-Vibes/ecorp.git',
  )
  const wrong = parseFixtureSourceIdentity('git@github.com:acme/widget.git')
  assert.equal(legacy.repository, 'shyamsridhar123/ecorp')
  assert.equal(current.repository, transferred.repository)
  assert.equal(wrong.repository, 'acme/widget')
  assert.equal(new Set([legacy.repository, current.repository, wrong.repository]).size, 3)
})

test('retains valid repository punctuation', () => {
  assert.equal(
    parseFixtureSourceIdentity('https://github.com/Acme/example_repo.v2.git').name,
    'example_repo.v2',
  )
})

const canary = 'fixture-secret-canary'
for (const [name, origin] of [
  ['missing', undefined],
  ['null', null],
  ['non-string', 42],
  ['empty', ''],
  ['bare namespace', 'All-The-Vibes/ecorp'],
  ['different host', 'https://example.com/All-The-Vibes/ecorp.git'],
  ['host suffix', 'https://github.com.example.com/All-The-Vibes/ecorp.git'],
  ['HTTP', 'http://github.com/All-The-Vibes/ecorp.git'],
  ['Git protocol', 'git://github.com/All-The-Vibes/ecorp.git'],
  ['HTTPS userinfo', `https://${canary}@github.com/All-The-Vibes/ecorp.git`],
  ['HTTPS password', `https://user:${canary}@github.com/All-The-Vibes/ecorp.git`],
  ['SSH password', `ssh://git:${canary}@github.com/All-The-Vibes/ecorp.git`],
  ['non-Git SSH user', `ssh://${canary}@github.com/All-The-Vibes/ecorp.git`],
  ['SCP userinfo', `${canary}@github.com:All-The-Vibes/ecorp.git`],
  ['SCP prefix not recognized by the runner', 'git@GITHUB.COM:All-The-Vibes/ecorp.git'],
  ['query', `https://github.com/All-The-Vibes/ecorp.git?token=${canary}`],
  ['fragment', `https://github.com/All-The-Vibes/ecorp.git#${canary}`],
  ['extra path', 'https://github.com/All-The-Vibes/ecorp/tree/main'],
  ['traversal', 'https://github.com/All-The-Vibes/ignored/../ecorp.git'],
  ['dot owner', 'git@github.com:../ecorp.git'],
  ['dot repository', 'git@github.com:All-The-Vibes/..'],
  ['empty repository', 'git@github.com:All-The-Vibes/.git'],
  ['encoded path', 'https://github.com/All-The-Vibes/%65corp.git'],
  ['backslash', 'https://github.com/All-The-Vibes\\ecorp.git'],
  ['multiple origins', 'https://github.com/acme/one.git\nhttps://github.com/acme/two.git'],
  ['oversized origin', `https://github.com/All-The-Vibes/${'x'.repeat(2048)}`],
]) {
  test(`rejects ${name} without disclosing the origin`, () => {
    assert.throws(
      () => parseFixtureSourceIdentity(origin),
      (error) => {
        assert.equal(
          error.message,
          'Fixture source requires a credential-free HTTPS or SSH github.com origin',
        )
        assert.equal(error.cause, undefined)
        assert.doesNotMatch(error.stack, new RegExp(canary))
        return true
      },
    )
  })
}

test('reads only the selected checkout origin with bounded, piped Git output', () => {
  const sourceRoot = '/owned fixture/source with spaces'
  const calls = []
  const identity = readFixtureSourceIdentity(sourceRoot, (command, args, options) => {
    calls.push({ command, args, options })
    return 'git@github.com:All-The-Vibes/ecorp.git\n'
  })
  assert.deepEqual(identity, transferred)
  assert.equal(calls.length, 1)
  assert.deepEqual(calls[0], {
    command: 'git',
    args: ['config', '--get', 'remote.origin.url'],
    options: {
      cwd: sourceRoot,
      encoding: 'utf8',
      windowsHide: true,
      timeout: 5000,
      maxBuffer: 4096,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  })
})

test('Git failure discards message, cause, stdout and stderr without fallback', () => {
  const unsafe = Object.assign(new Error(canary), {
    stdout: `https://${canary}@github.com/All-The-Vibes/ecorp.git`,
    stderr: canary,
  })
  assert.throws(
    () => readFixtureSourceIdentity('/owned/source', () => { throw unsafe }),
    (error) => {
      assert.equal(error.message, 'Unable to read fixture source origin with read-only Git')
      assert.equal(error.cause, undefined)
      assert.equal(error.stdout, undefined)
      assert.equal(error.stderr, undefined)
      assert.doesNotMatch(error.stack, new RegExp(canary))
      return true
    },
  )
})

test('a successful Git read cannot admit a credential-bearing origin', () => {
  assert.throws(
    () => readFixtureSourceIdentity(
      '/owned/source',
      () => `https://${canary}@github.com/All-The-Vibes/ecorp.git\n`,
    ),
    { message: 'Fixture source requires a credential-free HTTPS or SSH github.com origin' },
  )
})
