import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const read = file => readFileSync(new URL(file, import.meta.url), 'utf8').replaceAll('\r\n', '\n')

for (const file of ['e2e_factory_claims.mjs', 'e2e_factory_controller.mjs', 'e2e_factory_publication.mjs']) {
  test(file + ' uses the ATV checkout identity, including fixture URLs and policies', () => {
    const source = read('./' + file)
    assert.doesNotMatch(source, /shyamsridhar123/iu)
    assert.match(source, /all-the-vibes\/ecorp/u)
    assert.match(source, /https:\/\/github\.com\/all-the-vibes\/ecorp\/issues\//u)
    assert.match(source, /sourceBaseCommit/u)
  })
}

test('claims keep casing normalization, immutable-source assertions and pre-effect readiness on restart', () => {
  const source = read('./e2e_factory_claims.mjs')
  assert.match(source, /source_project_owner: 'AlL-tHe-ViBeS'/u)
  assert.match(source, /source_repository_owner: 'ALL-THE-VIBES'/u)
  assert.match(source, /mixedCaseReplay\.work_item\.source_repository_owner, 'all-the-vibes'/u)
  assert.match(source, /linkedTask\.contract\.source_repository, 'all-the-vibes\/ecorp'/u)
  assert.match(source, /linkedTask\.contract\.source_base_commit, sourceBaseCommit/u)
  assert.match(source, /selectFixtureRunnerForSource\(await snapshot\(demo\), demo/u)
  assert.match(source, /const initialReadinessPreviews = await waitForFixtureSource\(demo\)/u)
  assert.match(source, /await restartLocalServer\(\)\nconst restartReadinessPreviews = await waitForFixtureSource\(demo\)/u)
  assert.match(source, /assertAutomaticFactoryVerification\(completed\.state/u)
  assert.match(source, /assert\.equal\(staleVerified\.response\.status, 409\)/u)
  assert.match(source, /factory work item version is 5, not 4/u)
  assert.match(source, /'factory\.verified'/u)
  assert.doesNotMatch(source, /const verified = await postOk/u)
  for (const assertion of [
    'assert.equal(duplicateActive.response.status, 409)',
    'assert.equal(staleRenewal.response.status, 409)',
    'assert.equal(wrongToken.response.status, 409)',
    'assert.equal(forgedVerified.response.status, 409)',
    'assert.equal(forgedPublished.response.status, 400)',
  ]) assert.ok(source.includes(assertion), 'Lost fencing or authorization coverage: ' + assertion)
})

test('controller keeps its intentional different-repository failure and fake GitHub transport', () => {
  const source = read('./e2e_factory_controller.mjs')
  assert.match(source, /repository = 'All-The-Vibes\/ECorp'/u)
  assert.match(source, /fencedTask\.contract\.source_repository, 'all-the-vibes\/ecorp'/u)
  assert.match(source, /runs\[0\]\.source_repository, 'all-the-vibes\/ecorp'/u)
  assert.match(source, /task\.contract\.source_repository === 'acme\/widget'/u)
  assert.match(source, /source_base_commit === sourceBaseCommit/u)
  assert.match(source, /ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON\.stringify\(\[fakeGithub\]\)/u)
  assert.match(source, /ECORP_FAKE_GITHUB_STATE: statePath/u)
})

test('publication keeps mixed-case URLs, normalized effect keys, local bare transport and merge guards', () => {
  const source = read('./e2e_factory_publication.mjs')
  assert.match(source, /canonical_repository: 'All-The-Vibes\/ECorp'/u)
  assert.match(source, /https:\/\/github\.com\/All-The-Vibes\/ECorp\/pull\/41/u)
  assert.ok(source.includes('github-pr:${workItem.id}:${source.id}:all-the-vibes/ecorp:${branch}'))
  assert.match(source, /publication\.pull_request_head_repository_owner, 'all-the-vibes'/u)
  assert.match(source, /ECORP_PUBLICATION_TEST_REMOTE_URL: remotePath/u)
  assert.match(source, /ECORP_GITHUB_CLI_PREFIX_ARGS_JSON: JSON\.stringify\(\[fakeGithub\]\)/u)
  assert.match(source, /assert\.equal\(remoteMainAfter, remoteMainBefore\)/u)
  assert.match(source, /autoMergeRequest: null/u)
  assert.match(source, /assert\.equal\(collisionFakeState\.pr_create_calls, 0\)/u)
})

test('CI still runs all three full factory suites without suppressing failures', () => {
  const workflow = read('../.github/workflows/ci.yml')
  const integration = workflow.split('\n  integration:')[1].split('\n  external-adapters-windows:')[0]
  assert.match(workflow, /run: node --test .*tools\/ci_factory_repository\.test\.mjs/u)
  for (const file of ['claims', 'controller', 'publication']) {
    assert.ok(integration.includes('run: node tools/e2e_factory_' + file + '.mjs'))
  }
  assert.doesNotMatch(integration, /continue-on-error|CRONY_SKIP_SERVER_RESTART/u)
})
