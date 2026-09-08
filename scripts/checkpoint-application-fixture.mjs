// Deterministic application output for the native checkpoint recovery regression.
// This is fixture code, not a claim that a real vendor generated the application.
import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'

export const APPLICATION_ROOT = 'qa/issue195'

export const applicationFiles = Object.freeze({
  'app.mjs': String.raw`export const seed = [
  { id: 'sync', title: 'Investigate delayed order sync', priority: 'high', status: 'open' },
  { id: 'access', title: 'Review warehouse access', priority: 'normal', status: 'open' },
]

export function addIncident(incidents, { id, title, priority }) {
  const clean = String(title ?? '').trim()
  if (!clean || clean.length > 120) throw new Error('Enter a title of 1–120 characters.')
  if (!['critical', 'high', 'normal'].includes(priority)) throw new Error('Choose a priority.')
  if (!id || incidents.some((incident) => incident.id === id)) throw new Error('Incident ID must be unique.')
  return [...incidents, { id, title: clean, priority, status: 'open' }]
}

export function changeStatus(incidents, id, status) {
  if (!['open', 'resolved'].includes(status)) throw new Error('Unknown status.')
  if (!incidents.some((incident) => incident.id === id)) throw new Error('Incident not found.')
  return incidents.map((incident) => incident.id === id ? { ...incident, status } : incident)
}

export function visibleIncidents(incidents, query = '', status = 'all') {
  const needle = query.trim().toLowerCase()
  return incidents.filter((incident) => incident.title.toLowerCase().includes(needle)
    && (status === 'all' || incident.status === status))
}
`,
  'app.test.mjs': String.raw`import test from 'node:test'
import assert from 'node:assert/strict'
import { seed, addIncident, changeStatus, visibleIncidents } from './app.mjs'

test('creates a trimmed incident without changing the original list', () => {
  const next = addIncident(seed, { id: 'new', title: '  Shipping delay  ', priority: 'critical' })
  assert.equal(next.length, 3)
  assert.equal(next[2].title, 'Shipping delay')
  assert.equal(next[2].status, 'open')
  assert.equal(seed.length, 2)
})
test('rejects invalid incident input and duplicate identity', () => {
  for (const title of ['', '   ', 'x'.repeat(121)]) {
    assert.throws(() => addIncident(seed, { id: 'new', title, priority: 'normal' }))
  }
  assert.throws(() => addIncident(seed, { id: 'sync', title: 'Duplicate', priority: 'normal' }))
  assert.throws(() => addIncident(seed, { id: 'new', title: 'Bad priority', priority: 'unknown' }))
})
test('resolves and reopens the exact incident without modifying another incident', () => {
  const resolved = changeStatus(seed, 'sync', 'resolved')
  assert.equal(resolved[0].status, 'resolved')
  assert.equal(resolved[1], seed[1])
  assert.equal(seed[0].status, 'open')
  assert.equal(changeStatus(resolved, 'sync', 'open')[0].status, 'open')
  assert.throws(() => changeStatus(seed, 'absent', 'resolved'))
  assert.throws(() => changeStatus(seed, 'sync', 'deleted'))
})
test('filters by text and status together', () => {
  const resolved = changeStatus(seed, 'sync', 'resolved')
  assert.deepEqual(visibleIncidents(resolved, ' ORDER ', 'resolved').map((item) => item.id), ['sync'])
  assert.deepEqual(visibleIncidents(resolved, 'order', 'open'), [])
  assert.equal(visibleIncidents(resolved).length, 2)
})
`,
  'index.html': String.raw`<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Incident desk · ECorp checkpoint fixture</title>
<style>
:root { color-scheme: light; font: 17px/1.5 system-ui,sans-serif; color:#182d34; background:#f4f6f2; }
* { box-sizing:border-box; } body { margin:0; } main { max-width:920px; margin:auto; padding:36px 20px; }
header { border-bottom:3px solid #286052; margin-bottom:24px; } h1 { font-size:2rem; margin:4px 0; }
small { color:#435c61; } section { margin-top:26px; } h2 { font-size:1.2rem; }
form,.filters { display:flex; flex-wrap:wrap; gap:16px; align-items:end; }
label { display:flex; flex-direction:column; gap:6px; flex:1; min-width:180px; font-weight:600; }
input,select,button { font:inherit; min-height:44px; padding:9px 12px; border:1px solid #637f7a; border-radius:5px; background:white; color:inherit; }
button { cursor:pointer; font-weight:650; } button[type=submit] { background:#245c4d; color:white; }
:focus-visible { outline:3px solid #a23d19; outline-offset:3px; }
ul { padding:0; list-style:none; } li { display:flex; gap:16px; align-items:center; padding:18px 0; border-bottom:1px solid #b9cbc3; }
.info { flex:1; min-width:0; overflow-wrap:anywhere; } .info strong,.info small { display:block; }
#notice { min-height:1.5em; } [role=alert] { color:#942b21; } footer { margin-top:36px; color:#435c61; font-size:.85rem; }
@media(max-width:480px) { main { padding:20px 16px; } h1 { font-size:1.7rem; } label { min-width:100%; } }
</style>
<main>
  <header><small>ECORP · RECOVERY TEST APPLICATION</small><h1>Incident desk</h1>
    <p>A small, working incident tracker. Add, find, resolve and reopen issues.</p></header>
  <p id="summary" aria-live="polite"></p>
  <section aria-labelledby="create-title"><h2 id="create-title">Log an incident</h2>
    <form id="create">
      <label>Incident title<input name="title" maxlength="120" required autocomplete="off"></label>
      <label>Priority<select name="priority"><option value="normal">Normal</option><option value="high">High</option><option value="critical">Critical</option></select></label>
      <button type="submit">Add incident</button>
    </form><p id="notice" role="status"></p>
  </section>
  <section aria-labelledby="queue-title"><h2 id="queue-title">Incident queue</h2>
    <div class="filters">
      <label>Search incidents<input id="search" type="search" autocomplete="off"></label>
      <label>Show status<select id="status"><option value="all">All incidents</option><option value="open">Open</option><option value="resolved">Resolved</option></select></label>
    </div><ul id="incidents" aria-label="Incidents"></ul>
  </section>
  <footer>Deterministic QA fixture · data stays in this browser · no external services.</footer>
</main>
<script type="module">
import { seed, addIncident, changeStatus, visibleIncidents } from './app.mjs'
const key = 'ecorp-checkpoint-incident-desk'
const notice = document.querySelector('#notice')
let incidents = structuredClone(seed)
try {
  const saved = JSON.parse(localStorage.getItem(key))
  if (Array.isArray(saved) && saved.every((item) => item && typeof item.id === 'string'
    && typeof item.title === 'string' && ['critical','high','normal'].includes(item.priority)
    && ['open','resolved'].includes(item.status))) incidents = saved
} catch { notice.textContent = 'Saved data could not be read. Showing the example incidents.' }
function save() {
  try { localStorage.setItem(key, JSON.stringify(incidents)) }
  catch { notice.textContent = 'Changes are available now but could not be saved in this browser.' }
}
function render() {
  const open = incidents.filter((item) => item.status === 'open').length
  document.querySelector('#summary').textContent = open + ' open · ' + (incidents.length - open) + ' resolved'
  const list = document.querySelector('#incidents')
  list.replaceChildren()
  const shown = visibleIncidents(incidents, document.querySelector('#search').value, document.querySelector('#status').value)
  if (!shown.length) { const empty = document.createElement('li'); empty.textContent = 'No matching incidents.'; list.append(empty) }
  for (const incident of shown) {
    const row = document.createElement('li')
    const info = document.createElement('div'); info.className = 'info'
    const title = document.createElement('strong'); title.textContent = incident.title
    const detail = document.createElement('small'); detail.textContent = incident.priority + ' priority · ' + incident.status
    const button = document.createElement('button'); button.type = 'button'
    button.textContent = incident.status === 'open' ? 'Resolve' : 'Reopen'
    button.setAttribute('aria-label', button.textContent + ' ' + incident.title)
    button.addEventListener('click', () => {
      incidents = changeStatus(incidents, incident.id, incident.status === 'open' ? 'resolved' : 'open')
      notice.textContent = incident.title + ' updated.'
      save(); render()
      document.querySelector('#search').focus()
    })
    info.append(title, detail); row.append(info, button); list.append(row)
  }
}
document.querySelector('#create').addEventListener('submit', (event) => {
  event.preventDefault()
  const form = event.currentTarget
  try {
    incidents = addIncident(incidents, { id: crypto.randomUUID(), title: form.elements.title.value, priority: form.elements.priority.value })
    notice.textContent = 'Incident added.'
    form.reset(); save(); render(); form.elements.title.focus()
  } catch (error) { notice.textContent = error.message }
})
document.querySelector('#search').addEventListener('input', render)
document.querySelector('#status').addEventListener('change', render)
render()
</script>
</html>
`,
})

export function writeCheckpointApplication(workspace) {
  const destination = path.join(workspace, APPLICATION_ROOT)
  mkdirSync(destination, { recursive: true })
  for (const [name, content] of Object.entries(applicationFiles)) {
    writeFileSync(path.join(destination, name), content, { encoding: 'utf8' })
  }
}
