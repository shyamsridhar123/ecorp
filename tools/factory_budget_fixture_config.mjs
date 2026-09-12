import assert from 'node:assert/strict'

export function fixtureMode(args) {
  const allowed = new Set(['--dry-run', '--execute', '--overrun', '--missing-checkpoint'])
  for (const arg of args) assert.ok(allowed.has(arg), `Unknown argument: ${arg}`)
  assert.ok(!(args.includes('--dry-run') && args.includes('--execute')),
    'Choose --dry-run or --execute, never both')
  const overrun = args.includes('--overrun')
  const missingCheckpoint = args.includes('--missing-checkpoint')
  assert.ok(!(overrun && missingCheckpoint),
    'Run missing-checkpoint and overrun as separate cases')
  return { execute: args.includes('--execute'), overrun, missingCheckpoint }
}

export function referenceSnapshotUrl(value, qaApi) {
  if (value === undefined) return null
  const url = new URL(value)
  const uuid = '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
  assert.ok(['http:', 'https:'].includes(url.protocol) &&
    ['127.0.0.1', '[::1]'].includes(url.hostname) && url.port,
  'Reference snapshot must use an explicit loopback address and port')
  assert.ok(!url.username && !url.password && !url.hash,
    'Reference snapshot cannot contain credentials or a fragment')
  assert.ok(new RegExp(`^/api/corps/${uuid}/snapshot$`).test(url.pathname),
    'Reference may only target a Corp snapshot')
  assert.deepEqual([...url.searchParams.keys()], ['actor_id'],
    'Reference snapshot requires exactly one actor_id and no other parameters')
  assert.match(url.searchParams.get('actor_id'), new RegExp(`^${uuid}$`),
    'Reference actor_id must be a UUID')
  assert.notEqual(url.port, new URL(qaApi).port,
    'Reference snapshot must not target the QA authority')
  return url.href
}
