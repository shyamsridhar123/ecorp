import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

// Source-contract regressions complement, rather than replace, the real browser
// interaction checks and existing request/preview/server authorization suites.
const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8')
const form = app.slice(app.indexOf('<form className="mission-form arcade-mission-form"'),
  app.indexOf('className={`mission-console'))
const custom = form.slice(form.indexOf('<label className="mission-run-toggle custom-verification-toggle">'),
  form.indexOf('{customVerification && !deterministicHarness ? ('))
const create = app.slice(app.indexOf('const createMission ='), app.indexOf('const launchMission ='))

test('verification checks do not change launch intent', () => {
  assert.match(custom, /setCustomVerification\(event\.target\.checked\)/)
  assert.doesNotMatch(custom, /setPauseAfterPlanning/)
  assert.match(form, /onChange=\{\(event\) => setPauseAfterPlanning\(event\.target\.checked\)\}/)
})

test('review navigation cannot turn its click into a form submission', () => {
  const controls = form.slice(form.indexOf('<div className="arcade-form-controls">'))
  const navigation = controls.slice(controls.indexOf('<button key="review-setup"'),
    controls.indexOf('Review and build'))
  assert.match(navigation, /type="button"/)
  assert.match(navigation, /event\.preventDefault\(\)/)
  assert.match(controls, /key="submit-mission"/)
})

test('ordinary application setup cannot silently fall back to the fake runtime', () => {
  const adapters = app.slice(app.indexOf('const availableAdapters ='), app.indexOf('const selectedActor ='))
  assert.match(adapters, /\.filter\(\(adapter\) => developerMode \|\| adapter\.name !== 'fake-process'\)/)
  assert.match(adapters, /\[data, selectedMissionSource, developerMode\]/)
  assert.match(form, /!enabled && effectiveMissionAdapter === 'fake-process'/)
})

test('composer uses two clear steps and starts with an empty goal', () => {
  assert.match(app, /\[missionTitle, setMissionTitle\] = useState\(''\)/)
  assert.match(form, /\['brief', '01', 'Describe & setup'\]/)
  assert.match(form, /\['proof', '02', 'Review & build'\]/)
  assert.doesNotMatch(form, /missionComposerStep === 'loadout'/)
})

test('advanced settings and detailed requirements are optional disclosures', () => {
  assert.match(form, /<details className="mission-advanced-options">/)
  assert.match(form, /Model, limits and output/)
  assert.match(form, /Detailed requirements/)
  for (const id of ['mission-budget', 'mission-deliverable', 'mission-model', 'mission-reasoning',
    'mission-write-scope', 'mission-repository']) {
    assert(app.includes(`id="${id}"`), `The real ${id} control must remain available`)
  }
})

test('closing setup preserves the draft and returns to existing mission context', () => {
  const close = app.slice(app.indexOf('const closeMissionComposer ='), app.indexOf('const createMission ='))
  assert.match(close, /setMissionComposerCollapsed\(true\)/)
  assert.doesNotMatch(close, /setMissionTitle|setMissionDescription|setSelectedMissionId|api</)
  assert.match(app, /missionComposerCollapsed && \(\s*<div\s*className=\{`mission-console/)
})

test('explicit links to existing work reveal it without erasing an open draft', () => {
  const navigation = app.slice(app.indexOf('const navigateToWorkspaceEntity ='), app.indexOf('const focus = (raw: string)'))
  assert.match(navigation, /setSelectedMissionId\(missionId\)\s+setMissionComposerCollapsed\(true\)/)
  assert.doesNotMatch(navigation, /setMissionTitle|setMissionDescription/)
  const floor = app.slice(app.indexOf('onMissions={(agentId)'), app.indexOf('onFactory={() =>', app.indexOf('onMissions={(agentId)')))
  assert.match(floor, /setMissionComposerCollapsed\(true\)/)
})

test('native creation and explicit held-plan behavior remain unchanged', () => {
  assert.match(create, /!selectedMissionSource/)
  assert.match(create, /!missionSourceConfirmed/)
  assert.match(create, /missionVerifierErrors\.length > 0/)
  assert.match(create, /body: currentMissionRequest\.body/)
  assert.match(create, /if \(!pauseAfterPlanning\)/)
  assert.match(create, /missions\/\$\{created\.mission_id\}\/launch/)
  assert.match(create, /setSelectedMissionId\(created\.mission_id\)/)
  assert.doesNotMatch(create, /preview.*(?:approved|ready)|quote.*(?:approved|ready)/i)
})

test('review keeps the exact source and current server-derived allocation visible', () => {
  assert.match(form, /className="mission-review-target"/)
  assert.match(form, /selectedMissionSource\.repository/)
  assert.match(form, /selectedMissionSource\.baseCommit/)
  assert.match(form, /<MissionAllocationPreview key=\{currentMissionRequest\.key\} scope=\{currentMissionRequest\}/)
  assert.match(app, /Boolean\(missionTitle\.trim\(\)\) && missionSourceConfirmed && !busy/)
})
