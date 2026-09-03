import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import net from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const scenarioRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const evidenceDirectory = join(scenarioRoot, "evidence");
const screenshotPath = join(evidenceDirectory, "browser.png");
const mobileScreenshotPath = join(evidenceDirectory, "browser-mobile.png");
const resultPath = join(evidenceDirectory, "browser-result.json");
const playwrightModule = process.env.ECORP_PLAYWRIGHT_MODULE;

if (!playwrightModule) {
  throw new Error("ECORP_PLAYWRIGHT_MODULE must point to the bundled Playwright module");
}

const { chromium } = require(playwrightModule);

function freePort() {
  return new Promise((resolvePort, reject) => {
    const probe = net.createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolvePort(port));
    });
  });
}

async function waitForHealth(baseUrl, child) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Server exited with code ${child.exitCode}`);
    try {
      const response = await fetch(`${baseUrl}/api/health`);
      if (response.ok) return response.json();
    } catch {
      // The child may not have bound its port yet.
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 50));
  }
  throw new Error("VendorGuard health check did not become ready");
}

async function stopChild(child) {
  if (child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolveExit) => child.once("exit", resolveExit)),
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error("VendorGuard did not stop after SIGTERM")), 5_000),
    ),
  ]);
}

const port = await freePort();
const temporaryDirectory = await mkdtemp(join(tmpdir(), "vendor-guard-browser-"));
const baseUrl = `http://127.0.0.1:${port}`;
const child = spawn(process.execPath, [join(scenarioRoot, "server.mjs")], {
  cwd: scenarioRoot,
  env: {
    ...process.env,
    PORT: String(port),
    VENDOR_GUARD_DATA_FILE: join(temporaryDirectory, "state.json"),
  },
  stdio: ["ignore", "pipe", "pipe"],
});

let browser;
let page;
let mobilePage;
let serverOutput = "";
const consoleErrors = [];
const expectedNetworkErrors = [];
const pageErrors = [];
const workflow = {
  tenantIsolation: false,
  requesterCreateSubmit: false,
  makerCheckerDenied: false,
  independentReview: false,
  staleVersionConflict: false,
  idempotentReplay: false,
  idempotencyConflict: false,
  riskExplanation: false,
  auditVerified: false,
};
let health = null;
let mobileHealth = null;
let viewport = null;
let mobileViewport = null;
let status = "failed";
let failure;

child.stdout.on("data", (chunk) => {
  serverOutput += chunk.toString();
});
child.stderr.on("data", (chunk) => {
  serverOutput += chunk.toString();
});

async function applyContext({ tenant, actor, role }) {
  await page.selectOption("#tenant", tenant);
  await page.fill("#actor", actor);
  await page.selectOption("#role", role);
  await Promise.all([
    page.waitForResponse(
      (response) => response.url().endsWith("/api/vendors") && response.request().method() === "GET",
    ),
    page.click("#apply-context"),
  ]);
}

let expectingHttpFailure = false;
async function exerciseExpectedHttpFailure(operation) {
  expectingHttpFailure = true;
  try {
    return await operation();
  } finally {
    await page.waitForTimeout(50);
    expectingHttpFailure = false;
  }
}

function captureDiagnostics(targetPage, viewportName) {
  targetPage.on("console", (message) => {
    if (message.type() !== "error") return;
    if (expectingHttpFailure && message.text().startsWith("Failed to load resource:")) {
      expectedNetworkErrors.push(message.text());
    } else {
      consoleErrors.push(`${viewportName}: ${message.text()}`);
    }
  });
  targetPage.on("pageerror", (error) => pageErrors.push(`${viewportName}: ${error.message}`));
}

function measureViewport(targetPage) {
  return targetPage.evaluate(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    horizontalOverflow: document.documentElement.scrollWidth > window.innerWidth,
  }));
}

try {
  await mkdir(evidenceDirectory, { recursive: true });
  health = await waitForHealth(baseUrl, child);
  assert.equal(health.status, "ok");

  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  page = await context.newPage();
  captureDiagnostics(page, "desktop");

  await page.goto(baseUrl, { waitUntil: "networkidle" });
  await page.fill("#vendor-name", "Northstar Data");
  await page.fill("#vendor-service", "Processes restricted customer identity records");
  await page.selectOption("#data-classification", "restricted");
  await page.selectOption("#criticality", "high");
  await page.fill("#annual-spend", "2000000");
  await page.check("#internet-exposure");
  await Promise.all([
    page.waitForResponse(
      (response) => response.url().endsWith("/api/vendors") && response.request().method() === "POST",
    ),
    page.click("#vendor-form button[type=submit]"),
  ]);
  const card = page.locator(".vendor-card").first();
  await card.waitFor();
  const vendorId = await card.getAttribute("data-vendor-id");
  assert.ok(vendorId);
  assert.equal(await card.locator(".risk-score").textContent(), "100");

  const staleResult = await exerciseExpectedHttpFailure(() =>
    page.evaluate(async ({ id }) => {
      const headers = {
        "Content-Type": "application/json",
        "X-Tenant-Id": "acme",
        "X-Actor-Id": "requester.alex",
        "X-Role": "requester",
      };
      const vendor = {
        name: "Northstar Data",
        service: "Processes restricted identity and payment records",
        expectedVersion: 1,
        assessment: {
          dataClassification: "restricted",
          internetExposure: true,
          criticality: "high",
          annualSpend: 2_000_000,
          securityReview: false,
        },
      };
      const first = await fetch(`/api/vendors/${id}`, {
        method: "PATCH",
        headers,
        body: JSON.stringify(vendor),
      });
      const stale = await fetch(`/api/vendors/${id}`, {
        method: "PATCH",
        headers,
        body: JSON.stringify(vendor),
      });
      return { first: first.status, stale: stale.status, staleBody: await stale.json() };
    }, { id: vendorId }),
  );
  assert.equal(staleResult.first, 200);
  assert.equal(staleResult.stale, 409);
  assert.equal(staleResult.staleBody.error.code, "VERSION_CONFLICT");
  workflow.staleVersionConflict = true;

  await page.click("#refresh");
  await page.waitForTimeout(75);
  await page.locator(`[data-vendor-id="${vendorId}"] .submit-vendor`).click();
  await page.waitForFunction(
    (id) =>
      document.querySelector(`[data-vendor-id="${id}"] .status-pill`)?.textContent ===
      "pending review",
    vendorId,
  );
  workflow.requesterCreateSubmit = true;

  const idempotencyResult = await exerciseExpectedHttpFailure(() =>
    page.evaluate(async ({ id }) => {
      const headers = {
        "Content-Type": "application/json",
        "X-Tenant-Id": "acme",
        "X-Actor-Id": "requester.alex",
        "X-Role": "requester",
        "Idempotency-Key": `ui-${id}-2`,
      };
      const replay = await fetch(`/api/vendors/${id}/submit`, {
        method: "POST",
        headers,
        body: JSON.stringify({ expectedVersion: 2 }),
      });
      const conflict = await fetch(`/api/vendors/${id}/submit`, {
        method: "POST",
        headers,
        body: JSON.stringify({ expectedVersion: 3 }),
      });
      return {
        replayStatus: replay.status,
        replayBody: await replay.json(),
        conflictStatus: conflict.status,
        conflictBody: await conflict.json(),
      };
    }, { id: vendorId }),
  );
  assert.equal(idempotencyResult.replayStatus, 200);
  assert.equal(idempotencyResult.replayBody.idempotentReplay, true);
  workflow.idempotentReplay = true;
  assert.equal(idempotencyResult.conflictStatus, 409);
  assert.equal(idempotencyResult.conflictBody.error.code, "IDEMPOTENCY_CONFLICT");
  workflow.idempotencyConflict = true;

  const details = page.locator(`[data-vendor-id="${vendorId}"] .risk-details`);
  await details.locator("summary").click();
  await details.locator(".factor-list").waitFor();
  assert.match(await details.textContent(), /restricted data contributes 35 points/i);
  assert.match(await details.textContent(), /high 70-100/i);
  workflow.riskExplanation = true;

  await applyContext({ tenant: "globex", actor: "requester.gina", role: "requester" });
  assert.equal(await page.locator(".vendor-card").count(), 0);
  assert.match(await page.locator(".empty").textContent(), /No vendor assessments/);
  workflow.tenantIsolation = true;

  await applyContext({ tenant: "acme", actor: "requester.alex", role: "reviewer" });
  const reviewCard = page.locator(`[data-vendor-id="${vendorId}"]`);
  await reviewCard.locator(".review-reason").fill("I created this record but should not approve it.");
  await exerciseExpectedHttpFailure(async () => {
    await reviewCard.locator(".approve-vendor").click();
    await page.waitForFunction(
      () => document.querySelector("#notice")?.textContent.includes("Maker-checker denied"),
    );
  });
  workflow.makerCheckerDenied = true;

  await applyContext({ tenant: "acme", actor: "reviewer.riley", role: "reviewer" });
  const independentCard = page.locator(`[data-vendor-id="${vendorId}"]`);
  await independentCard
    .locator(".review-reason")
    .fill("Security controls and compensating safeguards support approval.");
  await independentCard.locator(".approve-vendor").click();
  await page.waitForFunction(
    (id) =>
      document.querySelector(`[data-vendor-id="${id}"] .status-pill`)?.textContent === "approved",
    vendorId,
  );
  assert.match(await page.locator(`[data-vendor-id="${vendorId}"]`).textContent(), /reviewer\.riley/);
  workflow.independentReview = true;

  await applyContext({ tenant: "acme", actor: "admin.avery", role: "admin" });
  await page.click("#verify-audit");
  await page.waitForFunction(
    () => document.querySelector("#audit-result")?.textContent.startsWith("VALID"),
  );
  assert.match(await page.locator("#audit-result").textContent(), /events · head [a-f0-9]{64}/);
  workflow.auditVerified = true;

  viewport = await measureViewport(page);
  assert.equal(viewport.horizontalOverflow, false);
  assert.deepEqual(consoleErrors, []);
  assert.deepEqual(pageErrors, []);
  assert.equal(expectedNetworkErrors.length, 3);
  assert.ok(Object.values(workflow).every(Boolean));
  await page.screenshot({ path: screenshotPath, fullPage: true });

  mobileHealth = await waitForHealth(baseUrl, child);
  assert.equal(mobileHealth.status, "ok");
  const mobileContext = await browser.newContext({ viewport: { width: 390, height: 844 } });
  mobilePage = await mobileContext.newPage();
  captureDiagnostics(mobilePage, "mobile");
  await mobilePage.goto(baseUrl, { waitUntil: "networkidle" });
  await mobilePage.locator("h1").waitFor();
  assert.equal(await mobilePage.locator(".vendor-card").count(), 1);
  assert.equal(
    await mobilePage.locator(`[data-vendor-id="${vendorId}"] .status-pill`).textContent(),
    "approved",
  );
  assert.equal(await mobilePage.locator("#intake-panel").isVisible(), true);
  assert.ok(Object.values(workflow).every(Boolean));
  mobileViewport = await measureViewport(mobilePage);
  assert.deepEqual(
    { width: mobileViewport.width, height: mobileViewport.height },
    { width: 390, height: 844 },
  );
  assert.equal(mobileViewport.horizontalOverflow, false);
  assert.deepEqual(consoleErrors, []);
  assert.deepEqual(pageErrors, []);
  await mobilePage.screenshot({ path: mobileScreenshotPath, fullPage: true });
  status = "passed";
} catch (error) {
  failure = error;
  if (mobilePage) {
    try {
      mobileViewport = await measureViewport(mobilePage);
      await mobilePage.screenshot({ path: mobileScreenshotPath, fullPage: true });
    } catch {
      // Preserve the primary browser workflow failure.
    }
  } else if (page) {
    try {
      viewport = await measureViewport(page);
      await page.screenshot({ path: screenshotPath, fullPage: true });
    } catch {
      // Preserve the primary browser workflow failure.
    }
  }
} finally {
  const result = {
    status,
    health,
    mobileHealth,
    workflow,
    consoleErrors,
    pageErrors,
    viewport,
    mobileViewport,
    expectedNetworkErrors,
    ...(failure ? { failure: failure.stack ?? String(failure), serverOutput } : {}),
  };
  await mkdir(evidenceDirectory, { recursive: true });
  await writeFile(resultPath, `${JSON.stringify(result, null, 2)}\n`, "utf8");
  if (browser) await browser.close();
  await stopChild(child);
  await rm(temporaryDirectory, { recursive: true, force: true });
}

if (failure) throw failure;
console.log(`Browser workflow passed; evidence written to ${resultPath}`);
