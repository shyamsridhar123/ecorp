// Trusted runner verifier, not a provider tool. Requires a task Git worktree.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { lstat, mkdir, readFile, realpath, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const root = await realpath(process.cwd())
assert.ok((await lstat(path.join(root, '.git'))).isFile(),
  'Run only in an isolated Git worktree, never the source checkout')
const directory = process.argv[2] ?? 'arcade'
assert.match(directory, /^[a-zA-Z0-9_-]+$/, 'One literal application directory is required')
const app = path.join(root, directory)
assert.ok(!(await lstat(app)).isSymbolicLink(), 'Application directory must not be linked')
assert.equal(await realpath(app), app)
const modulePath = process.env.CRONY_PLAYWRIGHT_MODULE
assert.ok(modulePath, 'The trusted runner must supply its installed Playwright module path')
const { chromium } = await import(pathToFileURL(path.join(modulePath, 'index.mjs')).href)
const evidence = path.join(app, 'evidence')
await mkdir(evidence, { recursive: true })
assert.ok(!(await lstat(evidence)).isSymbolicLink(), 'Evidence directory must not be linked')
assert.equal(await realpath(evidence), evidence)

async function outputPath(name) {
  const target = path.join(evidence, name)
  try {
    const info = await lstat(target)
    assert.ok(info.isFile() && !info.isSymbolicLink() && info.nlink === 1,
      'Refuse to overwrite a linked or non-regular evidence file')
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
  }
  return target
}

const browser = await chromium.launch({ channel: 'chrome', headless: true })
const report = { verifier: 'ecorp-arcade-browser-v1', passed: false, cases: [], errors: [], screenshots: [] }
try {
  for (const [name, width, height, reducedMotion] of [
    ['desktop', 1280, 800, 'no-preference'],
    ['mobile', 390, 844, 'reduce'],
  ]) {
    const page = await browser.newPage({ viewport: { width, height }, reducedMotion })
    const errors = []
    const remoteRequests = []
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text())
    })
    page.on('request', (request) => {
      if (/^https?:/u.test(request.url())) remoteRequests.push(request.url())
    })
    await page.route(/^https?:\/\//u, (route) => route.abort('blockedbyclient'))
    await page.goto(pathToFileURL(path.join(app, 'index.html')).href, { waitUntil: 'load' })
    const state = page.getByTestId('game-state')
    await state.waitFor({ state: 'visible' })
    assert.match(await state.innerText(), /ready/iu)
    const start = page.getByRole('button', { name: /^start(?: game)?$/iu })
    await start.focus()
    await page.keyboard.press('Enter')
    await page.waitForFunction(() =>
      /playing/i.test(document.querySelector('[data-testid="game-state"]')?.textContent ?? ''))
    const pause = page.getByRole('button', { name: /^pause(?: game)?$/iu })
    await pause.focus()
    await page.keyboard.press('Space')
    await page.waitForFunction(() =>
      /paused/i.test(document.querySelector('[data-testid="game-state"]')?.textContent ?? ''))
    const pauseState = await state.innerText()
    await page.getByRole('button', { name: /^resume(?: game)?$/iu }).focus()
    await page.keyboard.press('Enter')
    await page.waitForFunction(() =>
      /playing/i.test(document.querySelector('[data-testid="game-state"]')?.textContent ?? ''))
    await page.keyboard.press('ArrowRight')
    const restart = page.getByRole('button', { name: /^restart(?: game)?$/iu })
    await restart.focus()
    await page.keyboard.press('Space')
    await page.waitForFunction(() =>
      /ready|playing/i.test(document.querySelector('[data-testid="game-state"]')?.textContent ?? ''))
    const layout = await page.evaluate(() => ({
      width: innerWidth,
      scrollWidth: document.documentElement.scrollWidth,
      reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      focusedButton: document.activeElement?.tagName === 'BUTTON',
      canvas: document.querySelectorAll('canvas').length,
    }))
    assert.ok(layout.scrollWidth <= width, `${name} has horizontal overflow`)
    assert.ok(layout.focusedButton, 'Keyboard activation must retain semantic button focus')
    assert.equal(layout.reducedMotion, reducedMotion === 'reduce')
    assert.deepEqual(errors, [], `${name} has runtime errors`)
    assert.deepEqual(remoteRequests, [], 'The game must not request remote resources')
    const screenshot = await outputPath(`${name}.png`)
    await page.screenshot({ path: screenshot, fullPage: true })
    const bytes = await readFile(screenshot)
    report.screenshots.push({
      path: `${directory}/evidence/${name}.png`,
      sha256: createHash('sha256').update(bytes).digest('hex'),
      bytes: bytes.length,
    })
    report.cases.push({ name, width, height, reducedMotion, pauseState,
      finalState: await state.innerText(), layout, errors, remoteRequestCount: remoteRequests.length })
    await page.close()
  }
  report.passed = true
} catch (error) {
  report.errors.push(error.message)
  throw error
} finally {
  await browser.close()
  await writeFile(await outputPath('browser-verification.json'), `${JSON.stringify(report, null, 2)}\n`)
  console.log(JSON.stringify(report))
}
