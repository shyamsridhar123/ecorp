import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const scenarioRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const evidenceDirectory = join(scenarioRoot, 'evidence');
const resultPath = join(evidenceDirectory, 'browser-result.json');
const playwrightModule = process.env.ECORP_PLAYWRIGHT_MODULE;

if (!playwrightModule) {
  throw new Error('ECORP_PLAYWRIGHT_MODULE must point to the bundled Playwright module');
}

const { chromium } = require(playwrightModule);

function freePort() {
  return new Promise((resolvePort, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      server.close(() => resolvePort(port));
    });
  });
}

async function waitForHealth(baseUrl, child) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Server exited with code ${child.exitCode}`);
    try {
      const response = await fetch(`${baseUrl}/health`);
      if (response.ok) {
        const body = await response.json();
        return {
          status: body.status,
          service: body.service,
          persistence: body.persistence,
          sseConnections: body.sseConnections,
        };
      }
    } catch {
      // The server may not have bound its port yet.
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error('Incident Command health check did not become ready');
}

async function stopChild(child) {
  if (child.exitCode !== null) return;
  child.kill('SIGTERM');
  await Promise.race([
    new Promise((resolveExit) => child.once('exit', resolveExit)),
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error('Incident Command did not stop after SIGTERM')), 5_000),
    ),
  ]);
}

function isApiResponse(response, pathname, method) {
  return (
    new URL(response.url()).pathname === pathname &&
    response.request().method() === method
  );
}

async function measureViewport(page) {
  return page.evaluate(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    horizontalOverflow: document.documentElement.scrollWidth > window.innerWidth,
  }));
}

const port = await freePort();
const dataDirectory = await mkdtemp(join(tmpdir(), 'incident-command-browser-'));
const baseUrl = `http://127.0.0.1:${port}`;
const child = spawn(
  process.execPath,
  [join(scenarioRoot, 'server.mjs'), '--port', String(port), '--data-dir', dataDirectory],
  { cwd: scenarioRoot, stdio: ['ignore', 'pipe', 'pipe'] },
);

let browser;
let page;
let mobilePage;
let serverOutput = '';
let failure;
let health;
let mobileHealth;
let viewport;
let mobileViewport;
let status = 'failed';
const consoleErrors = [];
const pageErrors = [];
const sse = {
  connection: false,
  tenantFiltering: false,
  draftPreservingRefresh: false,
};
const workflow = {
  incidentDeclared: false,
  commanderWorkflow: false,
  responderTimeline: false,
  sseConnected: false,
  sseRefreshPreservedDraft: false,
  postmortemReloadPersistence: false,
  deterministicPostmortem: false,
  auditVerified: false,
  tenantIsolation: false,
  desktopResponsive: false,
  mobileResponsive: false,
};

child.stdout.on('data', (chunk) => {
  serverOutput += chunk.toString();
});
child.stderr.on('data', (chunk) => {
  serverOutput += chunk.toString();
});

function captureDiagnostics(target, label) {
  target.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(`${label}: ${message.text()}`);
  });
  target.on('pageerror', (error) => pageErrors.push(`${label}: ${error.message}`));
}

async function applyIdentity(target, { tenantId, actorId, role }) {
  await target.fill('#tenant-id', tenantId);
  await target.fill('#actor-id', actorId);
  await target.selectOption('#role', role);
  await Promise.all([
    target.waitForResponse((response) => isApiResponse(response, '/api/incidents', 'GET')),
    target.click('#identity-form button[type=submit]'),
  ]);
}

async function selectIncident(target, title) {
  const card = target.locator('.incident-card').filter({ hasText: title }).first();
  await card.waitFor();
  await card.click();
  await target.locator('#incident-detail:not(.hidden)').waitFor();
}

async function submitMutation(target, selector, pathname, method) {
  await Promise.all([
    target.waitForResponse((response) => isApiResponse(response, pathname, method)),
    target.click(selector),
  ]);
}

try {
  await mkdir(evidenceDirectory, { recursive: true });
  health = await waitForHealth(baseUrl, child);
  assert.equal(health.status, 'ok');

  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  page = await context.newPage();
  captureDiagnostics(page, 'desktop');
  await page.goto(baseUrl, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(
    () => document.querySelector('#stream-status')?.textContent === 'Live tenant stream connected',
  );
  sse.connection = true;
  workflow.sseConnected = true;

  const title = 'Checkout authorization failures';
  await page.fill('#create-form [name=title]', title);
  await page.selectOption('#create-form [name=severity]', 'sev1');
  await page.fill('#create-form [name=affectedService]', 'checkout-api');
  await page.fill(
    '#create-form [name=customerImpact]',
    'Customers cannot complete purchases in the primary region.',
  );
  const createResponsePromise = page.waitForResponse((response) =>
    isApiResponse(response, '/api/incidents', 'POST'),
  );
  await page.click('#create-form button[type=submit]');
  const incidentId = (await (await createResponsePromise).json()).incident.id;
  await page.waitForFunction(
    (id) => document.querySelector('#detail-id')?.textContent === id,
    incidentId,
  );
  workflow.incidentDeclared = true;

  await applyIdentity(page, {
    tenantId: 'acme-ops',
    actorId: 'casey.commander',
    role: 'commander',
  });
  await selectIncident(page, title);
  await page.fill('#assignment-form [name=owner]', 'oncall.primary');
  await submitMutation(
    page,
    '#assignment-form button[type=submit]',
    `/api/incidents/${incidentId}/assignment`,
    'PATCH',
  );
  await page.fill('#transition-form [name=note]', 'Incident command activated.');
  await submitMutation(
    page,
    '#transition-button',
    `/api/incidents/${incidentId}/transitions`,
    'POST',
  );
  await page.waitForFunction(
    () => document.querySelector('#detail-state')?.textContent === 'Investigating',
  );

  await applyIdentity(page, {
    tenantId: 'acme-ops',
    actorId: 'riley.responder',
    role: 'responder',
  });
  await selectIncident(page, title);
  await page.fill(
    '#timeline-form [name=message]',
    'Database connection saturation isolated; capacity mitigation is in progress.',
  );
  await submitMutation(
    page,
    '#timeline-form button[type=submit]',
    `/api/incidents/${incidentId}/timeline`,
    'POST',
  );
  workflow.responderTimeline = true;

  await applyIdentity(page, {
    tenantId: 'acme-ops',
    actorId: 'casey.commander',
    role: 'commander',
  });
  await selectIncident(page, title);
  for (const expectedState of ['Mitigating', 'Resolved']) {
    await submitMutation(
      page,
      '#transition-button',
      `/api/incidents/${incidentId}/transitions`,
      'POST',
    );
    await page.waitForFunction(
      (stateName) => document.querySelector('#detail-state')?.textContent === stateName,
      expectedState,
    );
  }
  workflow.commanderWorkflow = true;

  const contributingFactors =
    'Connection pool limits were undersized.\nRegional failover capacity was not load-tested.';
  const correctiveActions =
    'Add saturation alerts and load tests.\nIncrease warm failover capacity.';
  await page.fill('#postmortem-form [name=contributingFactors]', contributingFactors);
  await page.fill('#postmortem-form [name=correctiveActions]', correctiveActions);

  const canaryResponse = await fetch(`${baseUrl}/api/incidents`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-tenant-id': 'acme-ops',
      'x-actor-id': 'sse.reporter',
      'x-role': 'reporter',
      'idempotency-key': 'browser-sse-canary',
    },
    body: JSON.stringify({
      title: 'SSE refresh canary',
      severity: 'sev4',
      affectedService: 'status-page',
      customerImpact: 'Internal validation only.',
    }),
  });
  assert.equal(canaryResponse.status, 201);
  await page.waitForFunction(
    () => document.querySelector('#incident-count')?.textContent.startsWith('2 incidents'),
  );
  assert.equal(
    await page.inputValue('#postmortem-form [name=contributingFactors]'),
    contributingFactors,
  );
  assert.equal(
    await page.inputValue('#postmortem-form [name=correctiveActions]'),
    correctiveActions,
  );
  sse.draftPreservingRefresh = true;
  workflow.sseRefreshPreservedDraft = true;

  await submitMutation(
    page,
    '#postmortem-form button[type=submit]',
    `/api/incidents/${incidentId}/postmortem`,
    'PATCH',
  );
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForFunction(
    () => document.querySelector('#stream-status')?.textContent === 'Live tenant stream connected',
  );
  await selectIncident(page, title);
  assert.equal(
    await page.inputValue('#postmortem-form [name=contributingFactors]'),
    contributingFactors,
  );
  assert.equal(
    await page.inputValue('#postmortem-form [name=correctiveActions]'),
    correctiveActions,
  );
  workflow.postmortemReloadPersistence = true;

  await submitMutation(
    page,
    '#transition-button',
    `/api/incidents/${incidentId}/transitions`,
    'POST',
  );
  await page.waitForFunction(
    () => document.querySelector('#detail-state')?.textContent === 'Postmortem Complete',
  );
  await page.click('#preview-button');
  await page.locator('#postmortem-preview:not(.hidden)').waitFor();
  const markdown = await page.textContent('#postmortem-preview');
  assert.match(markdown, /## Contributing factors/);
  assert.match(markdown, /Connection pool limits were undersized/);
  assert.match(markdown, /## Corrective actions/);
  assert.match(markdown, /Increase warm failover capacity/);
  assert.match(markdown, /Status: VALID/);
  workflow.deterministicPostmortem = true;
  assert.match(await page.textContent('#detail-audit'), /^Valid · 8 entries$/);
  assert.equal(await page.locator('#timeline-list > li').count(), 8);
  workflow.auditVerified = true;

  viewport = await measureViewport(page);
  assert.deepEqual({ width: viewport.width, height: viewport.height }, { width: 1280, height: 900 });
  assert.equal(viewport.horizontalOverflow, false);
  workflow.desktopResponsive = true;
  await page.screenshot({ path: join(evidenceDirectory, 'browser.png'), fullPage: true });

  await applyIdentity(page, {
    tenantId: 'globex-ops',
    actorId: 'audrey.auditor',
    role: 'auditor',
  });
  assert.equal(await page.locator('.incident-card').count(), 0);
  sse.tenantFiltering = true;
  workflow.tenantIsolation = true;

  mobileHealth = await waitForHealth(baseUrl, child);
  const mobileContext = await browser.newContext({ viewport: { width: 390, height: 844 } });
  mobilePage = await mobileContext.newPage();
  captureDiagnostics(mobilePage, 'mobile');
  await mobilePage.goto(baseUrl, { waitUntil: 'domcontentloaded' });
  await mobilePage.waitForFunction(
    () => document.querySelector('#stream-status')?.textContent === 'Live tenant stream connected',
  );
  await selectIncident(mobilePage, title);
  assert.equal(
    await mobilePage.inputValue('#postmortem-form [name=contributingFactors]'),
    contributingFactors,
  );
  assert.equal(await mobilePage.textContent('#detail-state'), 'Postmortem Complete');
  mobileViewport = await measureViewport(mobilePage);
  assert.deepEqual(
    { width: mobileViewport.width, height: mobileViewport.height },
    { width: 390, height: 844 },
  );
  assert.equal(mobileViewport.horizontalOverflow, false);
  workflow.mobileResponsive = true;
  await mobilePage.screenshot({
    path: join(evidenceDirectory, 'browser-mobile.png'),
    fullPage: true,
  });

  assert.ok(Object.values(workflow).every(Boolean));
  assert.deepEqual(consoleErrors, []);
  assert.deepEqual(pageErrors, []);
  status = 'passed';
} catch (error) {
  failure = error;
} finally {
  await mkdir(evidenceDirectory, { recursive: true });
  await writeFile(
    resultPath,
    `${JSON.stringify(
      {
        status,
        health,
        mobileHealth,
        sse,
        workflow,
        consoleErrors,
        pageErrors,
        viewport,
        mobileViewport,
        ...(failure ? { failure: failure.stack ?? String(failure), serverOutput } : {}),
      },
      null,
      2,
    )}\n`,
    'utf8',
  );
  if (browser) await browser.close();
  await stopChild(child);
  await rm(dataDirectory, { recursive: true, force: true });
}

if (failure) throw failure;
assert.equal((await readFile(resultPath, 'utf8')).includes('"status": "passed"'), true);
console.log(`Browser workflow passed; evidence written to ${resultPath}`);
