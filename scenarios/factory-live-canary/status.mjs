import { pathToFileURL } from 'node:url'

const CANONICAL = new Map([
  ['todo', 'Todo'],
  ['in progress', 'In Progress'],
  ['in review', 'In Review'],
  ['done', 'Done'],
])

export function normalizeStatus(value) {
  if (typeof value !== 'string') {
    throw new TypeError('status must be a string')
  }

  const normalized = value.trim().replace(/\s+/g, ' ').toLowerCase()
  const canonical = CANONICAL.get(normalized)
  if (!canonical) {
    throw new TypeError('invalid status')
  }
  return canonical
}

function main(argv = process.argv) {
  if (argv.length !== 3) {
    process.stderr.write('usage: node status.mjs <status>\n')
    process.exitCode = 2
    return
  }

  let canonical
  try {
    canonical = normalizeStatus(argv[2])
  } catch (error) {
    if (!(error instanceof TypeError)) {
      throw error
    }
    process.stderr.write('invalid status\n')
    process.exitCode = 2
    return
  }

  process.stdout.write(`${canonical}\n`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main()
}
