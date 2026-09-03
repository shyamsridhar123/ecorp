import { createHash, randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(fileURLToPath(import.meta.url));
const PUBLIC_ROOT = join(ROOT, "public");
const BODY_LIMIT = 64 * 1024;
const ROLES = new Set(["requester", "reviewer", "admin"]);
const DECISIONS = new Set(["approve", "reject", "request_changes"]);
const EDITABLE_STATES = new Set(["draft", "changes_requested"]);
const TERMINAL_STATES = new Set(["approved", "rejected"]);
const DATA_CLASSIFICATIONS = new Set(["public", "internal", "confidential", "restricted"]);
const CRITICALITIES = new Set(["low", "medium", "high"]);

const MIME_TYPES = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
};

export class AppError extends Error {
  constructor(status, code, message, details) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stable(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function cleanText(value, field, { min = 1, max = 200 } = {}) {
  if (typeof value !== "string") {
    throw new AppError(422, "VALIDATION_ERROR", `${field} must be a string`, { field });
  }
  const cleaned = value.trim();
  if (cleaned.length < min || cleaned.length > max) {
    throw new AppError(
      422,
      "VALIDATION_ERROR",
      `${field} must contain between ${min} and ${max} characters`,
      { field, min, max },
    );
  }
  return cleaned;
}

function boundedNumber(value, field, min, max) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) {
    throw new AppError(422, "VALIDATION_ERROR", `${field} must be between ${min} and ${max}`, {
      field,
      min,
      max,
    });
  }
  return value;
}

function oneOf(value, field, allowed) {
  if (!allowed.has(value)) {
    throw new AppError(422, "VALIDATION_ERROR", `${field} is invalid`, {
      field,
      allowed: [...allowed],
    });
  }
  return value;
}

function assessmentFrom(input) {
  const source = input?.assessment ?? input;
  if (!source || typeof source !== "object" || Array.isArray(source)) {
    throw new AppError(422, "VALIDATION_ERROR", "assessment must be an object", {
      field: "assessment",
    });
  }
  if (typeof source.internetExposure !== "boolean" || typeof source.securityReview !== "boolean") {
    throw new AppError(
      422,
      "VALIDATION_ERROR",
      "internetExposure and securityReview must be booleans",
      { fields: ["internetExposure", "securityReview"] },
    );
  }
  return {
    dataClassification: oneOf(
      source.dataClassification,
      "dataClassification",
      DATA_CLASSIFICATIONS,
    ),
    internetExposure: source.internetExposure,
    criticality: oneOf(source.criticality, "criticality", CRITICALITIES),
    annualSpend: boundedNumber(source.annualSpend, "annualSpend", 0, 10_000_000),
    securityReview: source.securityReview,
  };
}

export function calculateRisk(assessment) {
  const factors = [];
  const add = (key, label, points, explanation) =>
    factors.push({ key, label, points, explanation });

  const classificationPoints = {
    public: 0,
    internal: 10,
    confidential: 22,
    restricted: 35,
  }[assessment.dataClassification];
  add(
    "data_classification",
    "Data classification",
    classificationPoints,
    `${assessment.dataClassification} data contributes ${classificationPoints} points.`,
  );

  const exposurePoints = assessment.internetExposure ? 25 : 0;
  add(
    "internet_exposure",
    "Internet exposure",
    exposurePoints,
    assessment.internetExposure
      ? "Internet-facing access contributes 25 points."
      : "No internet-facing access contributes 0 points.",
  );

  const criticalityPoints = { low: 0, medium: 12, high: 25 }[assessment.criticality];
  add(
    "business_criticality",
    "Business criticality",
    criticalityPoints,
    `${assessment.criticality} criticality contributes ${criticalityPoints} points.`,
  );

  const spendPoints =
    assessment.annualSpend >= 1_000_000 ? 15 : assessment.annualSpend >= 100_000 ? 8 : 0;
  add(
    "annual_spend",
    "Annual spend",
    spendPoints,
    `Annual spend of $${assessment.annualSpend.toLocaleString("en-US")} contributes ${spendPoints} points.`,
  );

  const reviewPoints = assessment.securityReview ? 0 : 15;
  add(
    "security_review",
    "Security review",
    reviewPoints,
    assessment.securityReview
      ? "A completed security review contributes 0 points."
      : "A missing security review contributes 15 points.",
  );

  const score = Math.min(100, factors.reduce((total, factor) => total + factor.points, 0));
  const level = score >= 70 ? "high" : score >= 40 ? "medium" : "low";
  return {
    score,
    level,
    factors,
    thresholds: {
      low: "0-39",
      medium: "40-69",
      high: "70-100",
      makerCheckerRequired: level === "high",
    },
  };
}

function blankState() {
  return { schemaVersion: 1, vendors: {}, idempotency: {}, audit: [] };
}

class FileStore {
  constructor(dataFile) {
    this.dataFile = dataFile;
    this.state = blankState();
    this.queue = Promise.resolve();
  }

  async load() {
    await mkdir(dirname(this.dataFile), { recursive: true });
    try {
      const parsed = JSON.parse(await readFile(this.dataFile, "utf8"));
      if (
        parsed?.schemaVersion !== 1 ||
        typeof parsed.vendors !== "object" ||
        typeof parsed.idempotency !== "object" ||
        !Array.isArray(parsed.audit)
      ) {
        throw new Error("unsupported data schema");
      }
      this.state = parsed;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      await this.persist();
    }
  }

  async persist() {
    const temporary = `${this.dataFile}.${process.pid}.${randomUUID()}.tmp`;
    await writeFile(temporary, `${JSON.stringify(this.state, null, 2)}\n`, "utf8");
    await rename(temporary, this.dataFile);
  }

  async read(operation) {
    await this.queue;
    return operation(this.state);
  }

  async transaction(operation) {
    const run = this.queue.then(async () => {
      const before = structuredClone(this.state);
      try {
        const result = await operation(this.state);
        await this.persist();
        return structuredClone(result);
      } catch (error) {
        this.state = before;
        throw error;
      }
    });
    this.queue = run.catch(() => {});
    return run;
  }
}

function actorFrom(request) {
  const tenantId = request.headers["x-tenant-id"];
  const actorId = request.headers["x-actor-id"];
  const role = request.headers["x-role"];
  if (
    typeof tenantId !== "string" ||
    !/^[a-z0-9][a-z0-9-]{1,39}$/.test(tenantId) ||
    typeof actorId !== "string" ||
    !/^[a-zA-Z0-9][a-zA-Z0-9_.@-]{1,79}$/.test(actorId) ||
    typeof role !== "string" ||
    !ROLES.has(role)
  ) {
    throw new AppError(
      401,
      "AUTH_CONTEXT_REQUIRED",
      "Valid X-Tenant-Id, X-Actor-Id, and X-Role headers are required",
    );
  }
  return { tenantId, actorId, role };
}

function requireRole(actor, ...roles) {
  if (!roles.includes(actor.role)) {
    throw new AppError(403, "FORBIDDEN", `This operation requires role: ${roles.join(" or ")}`);
  }
}

function expectedVersion(input) {
  if (!Number.isSafeInteger(input?.expectedVersion) || input.expectedVersion < 1) {
    throw new AppError(422, "VALIDATION_ERROR", "expectedVersion must be a positive integer", {
      field: "expectedVersion",
    });
  }
  return input.expectedVersion;
}

function assertVersion(vendor, expected) {
  if (vendor.version !== expected) {
    throw new AppError(409, "VERSION_CONFLICT", "The vendor was changed by another actor", {
      expectedVersion: expected,
      currentVersion: vendor.version,
    });
  }
}

function tenantVendor(state, actor, id) {
  const vendor = state.vendors[id];
  if (!vendor || vendor.tenantId !== actor.tenantId) {
    throw new AppError(404, "NOT_FOUND", "Vendor not found");
  }
  return vendor;
}

function appendAudit(state, actor, action, vendor, details = {}) {
  const tenantEvents = state.audit.filter((event) => event.tenantId === actor.tenantId);
  const previous = tenantEvents.at(-1);
  const event = {
    id: randomUUID(),
    sequence: (previous?.sequence ?? 0) + 1,
    timestamp: new Date().toISOString(),
    tenantId: actor.tenantId,
    actorId: actor.actorId,
    role: actor.role,
    action,
    entityType: "vendor",
    entityId: vendor.id,
    entityVersion: vendor.version,
    details,
    previousHash: previous?.hash ?? null,
  };
  event.hash = sha256(stable(event));
  state.audit.push(event);
  return event;
}

function verifyTenantAudit(state, tenantId) {
  const events = state.audit.filter((event) => event.tenantId === tenantId);
  let previousHash = null;
  for (let index = 0; index < events.length; index += 1) {
    const event = events[index];
    const { hash, ...unsigned } = event;
    const expectedHash = sha256(stable(unsigned));
    if (
      event.sequence !== index + 1 ||
      event.previousHash !== previousHash ||
      hash !== expectedHash
    ) {
      return {
        valid: false,
        eventCount: events.length,
        brokenAt: event.id ?? null,
        checkedAt: new Date().toISOString(),
      };
    }
    previousHash = hash;
  }
  return {
    valid: true,
    eventCount: events.length,
    headHash: previousHash,
    brokenAt: null,
    checkedAt: new Date().toISOString(),
  };
}

async function readJson(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > BODY_LIMIT) {
      throw new AppError(413, "BODY_TOO_LARGE", `Request body exceeds ${BODY_LIMIT} bytes`);
    }
    chunks.push(chunk);
  }
  if (chunks.length === 0) return {};
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new AppError(400, "INVALID_JSON", "Request body must be valid JSON");
  }
}

function sendJson(response, status, payload, requestId, extraHeaders = {}) {
  const body = JSON.stringify(payload);
  response.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(body),
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
    "X-Request-Id": requestId,
    ...extraHeaders,
  });
  response.end(body);
}

function sendError(response, error, requestId) {
  const known = error instanceof AppError;
  const status = known ? error.status : 500;
  const payload = {
    error: {
      code: known ? error.code : "INTERNAL_ERROR",
      message: known ? error.message : "An unexpected error occurred",
      requestId,
      ...(known && error.details !== undefined ? { details: error.details } : {}),
    },
  };
  if (!known) console.error(`[${requestId}]`, error);
  sendJson(response, status, payload, requestId);
}

function publicVendor(vendor) {
  return structuredClone(vendor);
}

function createDomain(store) {
  return {
    list(actor) {
      return store.read((state) =>
        Object.values(state.vendors)
          .filter((vendor) => vendor.tenantId === actor.tenantId)
          .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))
          .map(publicVendor),
      );
    },

    get(actor, id) {
      return store.read((state) => publicVendor(tenantVendor(state, actor, id)));
    },

    create(actor, input) {
      requireRole(actor, "requester");
      const now = new Date().toISOString();
      const assessment = assessmentFrom(input);
      const vendor = {
        id: randomUUID(),
        tenantId: actor.tenantId,
        name: cleanText(input.name, "name", { min: 2, max: 120 }),
        service: cleanText(input.service, "service", { min: 2, max: 240 }),
        assessment,
        risk: calculateRisk(assessment),
        status: "draft",
        version: 1,
        createdBy: actor.actorId,
        submittedBy: null,
        createdAt: now,
        updatedAt: now,
        reviewHistory: [],
      };
      return store.transaction((state) => {
        state.vendors[vendor.id] = vendor;
        appendAudit(state, actor, "vendor.created", vendor, {
          status: vendor.status,
          riskScore: vendor.risk.score,
        });
        return publicVendor(vendor);
      });
    },

    update(actor, id, input) {
      requireRole(actor, "requester");
      const version = expectedVersion(input);
      return store.transaction((state) => {
        const vendor = tenantVendor(state, actor, id);
        assertVersion(vendor, version);
        if (!EDITABLE_STATES.has(vendor.status)) {
          throw new AppError(409, "INVALID_STATE", "Only draft or changes-requested vendors can be edited", {
            currentState: vendor.status,
          });
        }
        const assessment = assessmentFrom(input);
        vendor.name = cleanText(input.name, "name", { min: 2, max: 120 });
        vendor.service = cleanText(input.service, "service", { min: 2, max: 240 });
        vendor.assessment = assessment;
        vendor.risk = calculateRisk(assessment);
        vendor.version += 1;
        vendor.updatedAt = new Date().toISOString();
        appendAudit(state, actor, "vendor.updated", vendor, {
          status: vendor.status,
          riskScore: vendor.risk.score,
        });
        return publicVendor(vendor);
      });
    },

    submit(actor, id, input, idempotencyKey) {
      requireRole(actor, "requester");
      if (
        typeof idempotencyKey !== "string" ||
        !/^[a-zA-Z0-9_.:-]{8,128}$/.test(idempotencyKey)
      ) {
        throw new AppError(
          400,
          "IDEMPOTENCY_KEY_REQUIRED",
          "A valid Idempotency-Key header (8-128 characters) is required",
        );
      }
      const version = expectedVersion(input);
      const scope = `${actor.tenantId}:submit:${id}:${idempotencyKey}`;
      const fingerprint = sha256(stable({ expectedVersion: version }));
      return store.transaction((state) => {
        const prior = state.idempotency[scope];
        if (prior) {
          if (prior.fingerprint !== fingerprint) {
            throw new AppError(
              409,
              "IDEMPOTENCY_CONFLICT",
              "Idempotency key was already used with a different request",
            );
          }
          return { vendor: prior.response, replayed: true };
        }

        const vendor = tenantVendor(state, actor, id);
        assertVersion(vendor, version);
        if (!EDITABLE_STATES.has(vendor.status)) {
          throw new AppError(409, "INVALID_STATE", "Vendor cannot be submitted from its current state", {
            currentState: vendor.status,
          });
        }
        vendor.status = "pending_review";
        vendor.submittedBy = actor.actorId;
        vendor.version += 1;
        vendor.updatedAt = new Date().toISOString();
        appendAudit(state, actor, "vendor.submitted", vendor, {
          riskLevel: vendor.risk.level,
          makerCheckerRequired: vendor.risk.thresholds.makerCheckerRequired,
        });
        const response = publicVendor(vendor);
        state.idempotency[scope] = {
          fingerprint,
          response,
          createdAt: new Date().toISOString(),
        };
        return { vendor: response, replayed: false };
      });
    },

    review(actor, id, input) {
      requireRole(actor, "reviewer");
      const version = expectedVersion(input);
      const decision = oneOf(input.decision, "decision", DECISIONS);
      const reason = cleanText(input.reason, "reason", { min: 10, max: 1000 });
      return store.transaction((state) => {
        const vendor = tenantVendor(state, actor, id);
        assertVersion(vendor, version);
        if (vendor.status !== "pending_review") {
          throw new AppError(409, "INVALID_STATE", "Only pending vendors can be reviewed", {
            currentState: vendor.status,
          });
        }
        if (
          vendor.risk.thresholds.makerCheckerRequired &&
          vendor.submittedBy === actor.actorId
        ) {
          throw new AppError(
            403,
            "MAKER_CHECKER_REQUIRED",
            "A high-risk vendor must be reviewed by an actor other than its submitter",
          );
        }

        const nextStatus = {
          approve: "approved",
          reject: "rejected",
          request_changes: "changes_requested",
        }[decision];
        vendor.status = nextStatus;
        vendor.version += 1;
        vendor.updatedAt = new Date().toISOString();
        const review = {
          id: randomUUID(),
          decision,
          reason,
          actorId: actor.actorId,
          timestamp: vendor.updatedAt,
        };
        vendor.reviewHistory.push(review);
        appendAudit(state, actor, `vendor.review.${decision}`, vendor, { reason });
        return publicVendor(vendor);
      });
    },

    audit(actor) {
      requireRole(actor, "admin");
      return store.read((state) =>
        state.audit
          .filter((event) => event.tenantId === actor.tenantId)
          .map((event) => structuredClone(event)),
      );
    },

    verifyAudit(actor) {
      requireRole(actor, "admin");
      return store.read((state) => verifyTenantAudit(state, actor.tenantId));
    },
  };
}

async function serveStatic(pathname, response, requestId) {
  const relative = pathname === "/" ? "index.html" : pathname.slice(1);
  if (!["index.html", "styles.css", "app.js"].includes(relative)) {
    throw new AppError(404, "NOT_FOUND", "Resource not found");
  }
  const content = await readFile(join(PUBLIC_ROOT, relative));
  response.writeHead(200, {
    "Content-Type": MIME_TYPES[extname(relative)] ?? "application/octet-stream",
    "Content-Length": content.length,
    "Cache-Control": "no-store",
    "Content-Security-Policy":
      "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
    "Referrer-Policy": "no-referrer",
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "X-Request-Id": requestId,
  });
  response.end(content);
}

export async function createVendorGuardServer({
  dataFile = process.env.VENDOR_GUARD_DATA_FILE ?? join(ROOT, "data", "vendor-guard.json"),
} = {}) {
  const store = new FileStore(dataFile);
  await store.load();
  const domain = createDomain(store);

  const server = createServer(async (request, response) => {
    const requestId = randomUUID();
    try {
      const url = new URL(request.url, "http://localhost");
      const pathname = url.pathname;

      if (request.method === "GET" && pathname === "/api/health") {
        sendJson(response, 200, { status: "ok", service: "vendor-guard" }, requestId);
        return;
      }

      if (!pathname.startsWith("/api/")) {
        if (request.method !== "GET") throw new AppError(405, "METHOD_NOT_ALLOWED", "Method not allowed");
        await serveStatic(pathname, response, requestId);
        return;
      }

      const actor = actorFrom(request);
      let match;
      if (request.method === "GET" && pathname === "/api/vendors") {
        sendJson(response, 200, { vendors: await domain.list(actor) }, requestId);
      } else if (
        request.method === "GET" &&
        (match = pathname.match(/^\/api\/vendors\/([0-9a-f-]+)$/))
      ) {
        sendJson(response, 200, { vendor: await domain.get(actor, match[1]) }, requestId);
      } else if (request.method === "POST" && pathname === "/api/vendors") {
        sendJson(response, 201, { vendor: await domain.create(actor, await readJson(request)) }, requestId);
      } else if (
        request.method === "PATCH" &&
        (match = pathname.match(/^\/api\/vendors\/([0-9a-f-]+)$/))
      ) {
        sendJson(
          response,
          200,
          { vendor: await domain.update(actor, match[1], await readJson(request)) },
          requestId,
        );
      } else if (
        request.method === "POST" &&
        (match = pathname.match(/^\/api\/vendors\/([0-9a-f-]+)\/submit$/))
      ) {
        const result = await domain.submit(
          actor,
          match[1],
          await readJson(request),
          request.headers["idempotency-key"],
        );
        sendJson(
          response,
          result.replayed ? 200 : 202,
          { vendor: result.vendor, idempotentReplay: result.replayed },
          requestId,
          { "Idempotent-Replay": String(result.replayed) },
        );
      } else if (
        request.method === "POST" &&
        (match = pathname.match(/^\/api\/vendors\/([0-9a-f-]+)\/reviews$/))
      ) {
        sendJson(
          response,
          200,
          { vendor: await domain.review(actor, match[1], await readJson(request)) },
          requestId,
        );
      } else if (request.method === "GET" && pathname === "/api/audit") {
        sendJson(response, 200, { events: await domain.audit(actor) }, requestId);
      } else if (request.method === "GET" && pathname === "/api/audit/verify") {
        sendJson(response, 200, { verification: await domain.verifyAudit(actor) }, requestId);
      } else {
        throw new AppError(404, "NOT_FOUND", "API route not found");
      }
    } catch (error) {
      sendError(response, error, requestId);
    }
  });

  return { server, dataFile };
}

export async function listen({ port = Number(process.env.PORT ?? 3000), host = "127.0.0.1", dataFile } = {}) {
  const app = await createVendorGuardServer({ dataFile });
  await new Promise((resolve, reject) => {
    app.server.once("error", reject);
    app.server.listen(port, host, resolve);
  });
  return app;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const app = await listen();
  const address = app.server.address();
  console.log(`VendorGuard listening on http://127.0.0.1:${address.port}`);
  const close = () => app.server.close(() => process.exit(0));
  process.once("SIGINT", close);
  process.once("SIGTERM", close);
}
