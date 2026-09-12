import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { parseFixtureSourceIdentity } from './fixture_source_identity.mjs'

const read = file => readFileSync(new URL(file, import.meta.url), 'utf8').replaceAll('\r\n', '\n')

test('the checked-in fixture identity helper canonicalizes the intended ATV origin', () => {
  assert.deepEqual(parseFixtureSourceIdentity('https://github.com/All-The-Vibes/ecorp.git'),
    { owner: 'all-the-vibes', name: 'ecorp', repository: 'all-the-vibes/ecorp', url: 'https://github.com/all-the-vibes/ecorp' })
})

for (const file of ['e2e_factory_claims.mjs', 'e2e_factory_controller.mjs', 'e2e_factory_publication.mjs']) {
  test(file + ' derives URLs and policies from the actual checked Git origin', () => {
    const source = read('./' + file)
    assert.doesNotMatch(source, /shyamsridhar123/iu)
    assert.ok(source.includes('readFixtureSourceIdentity(sourceRoot)'))
    assert.ok(source.includes('fixtureSource.repository'))
    assert.ok(source.includes('fixtureSource.url'))
    assert.ok(source.includes('sourceBaseCommit'))
  })
}

test('claims retain normalized source, pre-effect readiness and native automatic verification', () => {
  const source = read('./e2e_factory_claims.mjs')
  for (const required of ['source_project_owner: fixtureSource.owner.toUpperCase()',
    'source_repository_owner: fixtureSource.owner.toUpperCase()',
    'mixedCaseReplay.work_item.source_repository_owner, fixtureSource.owner',
    'linkedTask.contract.source_repository, fixtureSource.repository',
    'linkedTask.contract.source_base_commit, sourceBaseCommit',
    'selectFixtureRunnerForSource(await snapshot(demo), demo',
    'const initialReadinessPreviews = await waitForFixtureSource(demo)',
    'const restartReadinessPreviews = await waitForFixtureSource(demo)',
    'assertAutomaticFactoryVerification(completed.state',
    'assert.equal(staleVerified.response.status, 409)', 'factory work item version is 5, not 4',
    'assert.equal(duplicateActive.response.status, 409)', 'assert.equal(staleRenewal.response.status, 409)',
    'assert.equal(wrongToken.response.status, 409)', 'assert.equal(forgedVerified.response.status, 409)',
    'assert.equal(forgedPublished.response.status, 400)']) assert.ok(source.includes(required), required)
  assert.doesNotMatch(source, /const verified = await postOk/u)
})

test('controller retains wrong-repository rejection, native attempt preview and fake GitHub transport', () => {
  const source = read('./e2e_factory_controller.mjs')
  for (const required of ['repository = fixtureSource.repository.toUpperCase()',
    'fencedTask.contract.source_repository, fixtureSource.repository',
    'runs[0].source_repository, fixtureSource.repository',
    "task.contract.source_repository === 'acme/widget'", 'source_base_commit === sourceBaseCommit',
    'max_attempts: 2', 'ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub])',
    'ECORP_FAKE_GITHUB_STATE: statePath']) assert.ok(source.includes(required), required)
})

test('publication retains dynamic casing/effect identity, local bare transport and merge guards', () => {
  const source = read('./e2e_factory_publication.mjs')
  for (const required of ['canonical_repository: fixtureSource.repository.toUpperCase()',
    'fixtureSource.repository.toUpperCase()}/pull/41',
    'github-pr:${workItem.id}:${source.id}:${fixtureSource.repository}:${branch}',
    'publication.pull_request_head_repository_owner, fixtureSource.owner',
    'ECORP_PUBLICATION_TEST_REMOTE_URL: remotePath', 'ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON.stringify([fakeGithub])',
    'assert.equal(remoteMainAfter, remoteMainBefore)', 'autoMergeRequest: null',
    'assert.equal(collisionFakeState.pr_create_calls, 0)']) assert.ok(source.includes(required), required)
})

test('CI runs every complete factory suite without suppressing errors or restart checks', () => {
  const workflow = read('../.github/workflows/ci.yml')
  const integration = workflow.split('\n  integration:')[1].split('\n  external-adapters-windows:')[0]
  assert.match(workflow, /run: node --test .*tools\/ci_factory_repository\.test\.mjs/u)
  for (const file of ['claims', 'controller', 'publication']) {
    assert.ok(integration.includes('run: node tools/e2e_factory_' + file + '.mjs'))
  }
  assert.doesNotMatch(integration, /continue-on-error|CRONY_SKIP_SERVER_RESTART/u)
})
