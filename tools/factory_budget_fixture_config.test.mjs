import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureMode, referenceSnapshotUrl } from './factory_budget_fixture_config.mjs'

const qa = 'http://127.0.0.1:18450'
const route = '/api/corps/00000000-0000-4000-8000-000000000001/snapshot'
const query = '?actor_id=00000000-0000-4000-8000-000000000011'
const reference = `http://127.0.0.1:8791${route}${query}`

test('no mode is a dry-run; execution always requires its explicit switch', () => {
  assert.equal(fixtureMode([]).execute, false)
  assert.equal(fixtureMode(['--dry-run']).execute, false)
  assert.equal(fixtureMode(['--execute']).execute, true)
  assert.deepEqual(fixtureMode(['--dry-run', '--missing-checkpoint']),
    { execute: false, overrun: false, missingCheckpoint: true })
})

test('ambiguous execution modes and unknown switches fail closed', () => {
  for (const args of [
    ['--dry-run', '--execute'], ['--execute', '--dry-run'],
    ['--overrun', '--missing-checkpoint'], ['--excute'],
  ]) assert.throws(() => fixtureMode(args))
})

test('reference inspection is opt-in and supports one exact snapshot', () => {
  assert.equal(referenceSnapshotUrl(undefined, qa), null)
  assert.equal(referenceSnapshotUrl(reference, qa), reference)
})

test('reference URL rejects remote, credential-bearing, unrelated and QA targets', () => {
  for (const url of [
    '', reference.replace('127.0.0.1', 'example.com'),
    reference.replace('127.0.0.1', 'localhost'),
    reference.replace('http://', 'http://user:secret@'),
    reference.replace(':8791', ''), reference.replace('http:', 'ftp:'),
    `${reference}#fragment`, `${reference}&token=secret`, `${reference}&actor_id=duplicate`,
    reference.replace(query, ''), reference.replace(query, '?actor_id=invalid'),
    reference.replace('/snapshot', '/missions'),
    reference.replace('00000000-0000-4000-8000-000000000001', 'invalid'),
    `${qa}${route}${query}`,
    `http://[::1]:18450${route}${query}`,
  ]) assert.throws(() => referenceSnapshotUrl(url, qa))
})
