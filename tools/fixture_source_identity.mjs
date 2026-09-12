import { execFileSync } from 'node:child_process'

const invalidOrigin =
  'Fixture source requires a credential-free HTTPS or SSH github.com origin'

export function parseFixtureSourceIdentity(origin) {
  // Match the runner's GitHub namespace, not a redirect or a historical alias.
  // Strict authority/path syntax also rejects userinfo, tokens and URL escapes.
  if (typeof origin !== 'string' || origin.length > 2048) {
    throw new Error(invalidOrigin)
  }
  const remote = origin.trim()
  const match = remote.match(
    /^(?:https:\/\/github\.com(?::443)?\/|ssh:\/\/git@github\.com(?::(?:22|443))?\/)([a-z0-9_.-]+)\/([a-z0-9_.-]+)\/?$/i,
  ) ?? remote.match(/^git@github\.com:([a-zA-Z0-9_.-]+)\/([a-zA-Z0-9_.-]+)\/?$/)
  if (!match) throw new Error(invalidOrigin)
  const owner = match[1].toLowerCase()
  const name = match[2].replace(/(?:\.git)+$/, '').toLowerCase()
  if (!name || [owner, name].some((part) => part === '.' || part === '..')) {
    throw new Error(invalidOrigin)
  }
  const repository = `${owner}/${name}`
  return Object.freeze({
    owner,
    name,
    repository,
    url: `https://github.com/${repository}`,
  })
}

export function readFixtureSourceIdentity(sourceRoot, runGit = execFileSync) {
  let origin
  try {
    // Use the same read as the runner, without Git URL rewrites or remote I/O.
    origin = runGit('git', ['config', '--get', 'remote.origin.url'], {
      cwd: sourceRoot,
      encoding: 'utf8',
      windowsHide: true,
      timeout: 5000,
      maxBuffer: 4096,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
  } catch {
    // Child errors can contain credential-bearing stdout/stderr. Do not retain
    // the original error as a cause or interpolate the origin into diagnostics.
    throw new Error('Unable to read fixture source origin with read-only Git')
  }
  return parseFixtureSourceIdentity(origin)
}
