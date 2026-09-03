import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { calculateRisk, createVendorGuardServer } from "../server.mjs";

const REQUESTER = { tenant: "acme", actor: "requester.alex", role: "requester" };
const REVIEWER = { tenant: "acme", actor: "reviewer.riley", role: "reviewer" };
const ADMIN = { tenant: "acme", actor: "admin.avery", role: "admin" };

const HIGH_VENDOR = {
  name: "Northstar Data",
  service: "Processes restricted customer identity records",
  assessment: {
    dataClassification: "restricted",
    internetExposure: true,
    criticality: "high",
    annualSpend: 2_000_000,
    securityReview: false,
  },
};

const LOW_VENDOR = {
  name: "Paper Trail",
  service: "Delivers public office stationery catalogues",
  assessment: {
    dataClassification: "public",
    internetExposure: false,
    criticality: "low",
    annualSpend: 9_000,
    securityReview: true,
  },
};

function authHeaders(context, extra = {}) {
  return {
    "Content-Type": "application/json",
    "X-Tenant-Id": context.tenant,
    "X-Actor-Id": context.actor,
    "X-Role": context.role,
    ...extra,
  };
}

async function start(dataFile) {
  const app = await createVendorGuardServer({ dataFile });
  await new Promise((resolve, reject) => {
    app.server.once("error", reject);
    app.server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = app.server.address();
  return {
    ...app,
    baseUrl: `http://127.0.0.1:${port}`,
    close: () => new Promise((resolve) => app.server.close(resolve)),
  };
}

async function request(app, path, { context = REQUESTER, method = "GET", body, headers, raw } = {}) {
  const response = await fetch(`${app.baseUrl}${path}`, {
    method,
    headers: authHeaders(context, headers),
    body: raw ?? (body === undefined ? undefined : JSON.stringify(body)),
  });
  return {
    status: response.status,
    headers: response.headers,
    body: await response.json(),
  };
}

async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), "vendor-guard-test-"));
  const dataFile = join(directory, "state.json");
  const app = await start(dataFile);
  t.after(async () => {
    if (app.server.listening) await app.close();
    await rm(directory, { recursive: true, force: true });
  });
  return { app, dataFile, directory };
}

async function createVendor(app, input = HIGH_VENDOR, context = REQUESTER) {
  const result = await request(app, "/api/vendors", {
    method: "POST",
    context,
    body: input,
  });
  assert.equal(result.status, 201);
  return result.body.vendor;
}

test("risk scoring is deterministic and explains every bounded threshold", () => {
  const first = calculateRisk(HIGH_VENDOR.assessment);
  const second = calculateRisk(structuredClone(HIGH_VENDOR.assessment));
  assert.deepEqual(first, second);
  assert.equal(first.score, 100);
  assert.equal(first.level, "high");
  assert.equal(first.factors.length, 5);
  assert.deepEqual(first.thresholds, {
    low: "0-39",
    medium: "40-69",
    high: "70-100",
    makerCheckerRequired: true,
  });
  assert.equal(calculateRisk(LOW_VENDOR.assessment).score, 0);
});

test("tenant isolation and role authorization do not leak records", async (t) => {
  const { app } = await fixture(t);
  const vendor = await createVendor(app);
  const otherTenant = { tenant: "globex", actor: "requester.gina", role: "requester" };

  const isolatedList = await request(app, "/api/vendors", { context: otherTenant });
  assert.equal(isolatedList.status, 200);
  assert.deepEqual(isolatedList.body.vendors, []);

  const isolatedGet = await request(app, `/api/vendors/${vendor.id}`, { context: otherTenant });
  assert.equal(isolatedGet.status, 404);
  assert.equal(isolatedGet.body.error.code, "NOT_FOUND");

  const reviewerCreate = await request(app, "/api/vendors", {
    method: "POST",
    context: REVIEWER,
    body: LOW_VENDOR,
  });
  assert.equal(reviewerCreate.status, 403);
  assert.equal(reviewerCreate.body.error.code, "FORBIDDEN");

  const requesterAudit = await request(app, "/api/audit/verify");
  assert.equal(requesterAudit.status, 403);
  assert.equal(requesterAudit.body.error.code, "FORBIDDEN");
});

test("validation and malformed requests return bounded structured errors", async (t) => {
  const { app } = await fixture(t);
  const invalid = await request(app, "/api/vendors", {
    method: "POST",
    body: {
      ...HIGH_VENDOR,
      name: "x".repeat(121),
      assessment: { ...HIGH_VENDOR.assessment, annualSpend: 10_000_001 },
    },
  });
  assert.equal(invalid.status, 422);
  assert.equal(invalid.body.error.code, "VALIDATION_ERROR");
  assert.ok(invalid.body.error.requestId);

  const malformed = await request(app, "/api/vendors", {
    method: "POST",
    raw: "{broken",
  });
  assert.equal(malformed.status, 400);
  assert.equal(malformed.body.error.code, "INVALID_JSON");

  const missingAuth = await fetch(`${app.baseUrl}/api/vendors`);
  const missingPayload = await missingAuth.json();
  assert.equal(missingAuth.status, 401);
  assert.equal(missingPayload.error.code, "AUTH_CONTEXT_REQUIRED");
});

test("submission is idempotent and conflicting key reuse is rejected", async (t) => {
  const { app } = await fixture(t);
  const vendor = await createVendor(app);
  const path = `/api/vendors/${vendor.id}/submit`;
  const options = {
    method: "POST",
    body: { expectedVersion: 1 },
    headers: { "Idempotency-Key": "submit-northstar-001" },
  };

  const first = await request(app, path, options);
  assert.equal(first.status, 202);
  assert.equal(first.body.idempotentReplay, false);
  assert.equal(first.body.vendor.version, 2);

  const replay = await request(app, path, options);
  assert.equal(replay.status, 200);
  assert.equal(replay.body.idempotentReplay, true);
  assert.deepEqual(replay.body.vendor, first.body.vendor);
  assert.equal(replay.headers.get("idempotent-replay"), "true");

  const conflict = await request(app, path, {
    ...options,
    body: { expectedVersion: 2 },
  });
  assert.equal(conflict.status, 409);
  assert.equal(conflict.body.error.code, "IDEMPOTENCY_CONFLICT");
});

test("optimistic versions serialize concurrent edits and expose stale conflicts", async (t) => {
  const { app } = await fixture(t);
  const vendor = await createVendor(app, LOW_VENDOR);
  const path = `/api/vendors/${vendor.id}`;
  const update = {
    ...LOW_VENDOR,
    expectedVersion: 1,
    service: "Updated catalogue delivery service",
  };

  const results = await Promise.all([
    request(app, path, { method: "PATCH", body: update }),
    request(app, path, { method: "PATCH", body: update }),
  ]);
  assert.deepEqual(
    results.map((result) => result.status).sort(),
    [200, 409],
  );
  const conflict = results.find((result) => result.status === 409);
  assert.equal(conflict.body.error.code, "VERSION_CONFLICT");
  assert.equal(conflict.body.error.details.currentVersion, 2);
});

test("maker-checker, durable reasoning, resubmission, and terminal states are enforced", async (t) => {
  const { app } = await fixture(t);
  const vendor = await createVendor(app);
  const submitted = await request(app, `/api/vendors/${vendor.id}/submit`, {
    method: "POST",
    body: { expectedVersion: 1 },
    headers: { "Idempotency-Key": "maker-submit-001" },
  });
  assert.equal(submitted.status, 202);

  const sameActorReviewer = { ...REVIEWER, actor: REQUESTER.actor };
  const denied = await request(app, `/api/vendors/${vendor.id}/reviews`, {
    method: "POST",
    context: sameActorReviewer,
    body: {
      expectedVersion: 2,
      decision: "approve",
      reason: "I submitted this vendor and should be denied.",
    },
  });
  assert.equal(denied.status, 403);
  assert.equal(denied.body.error.code, "MAKER_CHECKER_REQUIRED");

  const changes = await request(app, `/api/vendors/${vendor.id}/reviews`, {
    method: "POST",
    context: REVIEWER,
    body: {
      expectedVersion: 2,
      decision: "request_changes",
      reason: "Attach the missing security review before approval.",
    },
  });
  assert.equal(changes.status, 200);
  assert.equal(changes.body.vendor.status, "changes_requested");
  assert.equal(changes.body.vendor.reviewHistory[0].reason, "Attach the missing security review before approval.");

  const updated = await request(app, `/api/vendors/${vendor.id}`, {
    method: "PATCH",
    body: {
      ...HIGH_VENDOR,
      expectedVersion: 3,
      assessment: { ...HIGH_VENDOR.assessment, securityReview: true },
    },
  });
  assert.equal(updated.status, 200);

  const resubmitted = await request(app, `/api/vendors/${vendor.id}/submit`, {
    method: "POST",
    body: { expectedVersion: 4 },
    headers: { "Idempotency-Key": "maker-submit-002" },
  });
  assert.equal(resubmitted.status, 202);

  const approved = await request(app, `/api/vendors/${vendor.id}/reviews`, {
    method: "POST",
    context: REVIEWER,
    body: {
      expectedVersion: 5,
      decision: "approve",
      reason: "Security evidence is attached and the residual risk is accepted.",
    },
  });
  assert.equal(approved.status, 200);
  assert.equal(approved.body.vendor.status, "approved");

  const terminalEdit = await request(app, `/api/vendors/${vendor.id}`, {
    method: "PATCH",
    body: { ...HIGH_VENDOR, expectedVersion: 6 },
  });
  assert.equal(terminalEdit.status, 409);
  assert.equal(terminalEdit.body.error.code, "INVALID_STATE");

  const terminalReview = await request(app, `/api/vendors/${vendor.id}/reviews`, {
    method: "POST",
    context: REVIEWER,
    body: {
      expectedVersion: 6,
      decision: "reject",
      reason: "A second terminal decision must not be accepted.",
    },
  });
  assert.equal(terminalReview.status, 409);
  assert.equal(terminalReview.body.error.code, "INVALID_STATE");
});

test("persistence survives restart and an admin can verify the SHA-256 chain", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "vendor-guard-restart-"));
  const dataFile = join(directory, "state.json");
  t.after(() => rm(directory, { recursive: true, force: true }));

  const firstApp = await start(dataFile);
  const vendor = await createVendor(firstApp, LOW_VENDOR);
  await firstApp.close();

  const secondApp = await start(dataFile);
  t.after(async () => {
    if (secondApp.server.listening) await secondApp.close();
  });
  const restored = await request(secondApp, `/api/vendors/${vendor.id}`);
  assert.equal(restored.status, 200);
  assert.equal(restored.body.vendor.name, LOW_VENDOR.name);

  const verification = await request(secondApp, "/api/audit/verify", { context: ADMIN });
  assert.equal(verification.status, 200);
  assert.equal(verification.body.verification.valid, true);
  assert.equal(verification.body.verification.eventCount, 1);
  assert.match(verification.body.verification.headHash, /^[a-f0-9]{64}$/);
});

test("audit verification detects persisted event tampering", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "vendor-guard-tamper-"));
  const dataFile = join(directory, "state.json");
  t.after(() => rm(directory, { recursive: true, force: true }));

  const firstApp = await start(dataFile);
  await createVendor(firstApp);
  await firstApp.close();

  const persisted = JSON.parse(await readFile(dataFile, "utf8"));
  persisted.audit[0].details.riskScore = 1;
  await writeFile(dataFile, `${JSON.stringify(persisted, null, 2)}\n`, "utf8");

  const secondApp = await start(dataFile);
  t.after(async () => {
    if (secondApp.server.listening) await secondApp.close();
  });
  const verification = await request(secondApp, "/api/audit/verify", { context: ADMIN });
  assert.equal(verification.status, 200);
  assert.equal(verification.body.verification.valid, false);
  assert.equal(verification.body.verification.brokenAt, persisted.audit[0].id);
});
