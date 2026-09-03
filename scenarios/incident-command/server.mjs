import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { randomUUID } from 'node:crypto';
import {
  AppError,
  ROLES,
  SEVERITIES,
  STATES,
  appendTechnicalUpdate,
  assignIncident,
  buildPostmortemMarkdown,
  canonicalJson,
  changeSeverity,
  createIncidentRecord,
  postmortemFilename,
  presentIncident,
  sha256,
  transitionIncident,
  updatePostmortem,
  validateAssignmentPayload,
  validateCreatePayload,
  validateIncidentId,
  validatePostmortemPayload,
  validateSeverityPayload,
  validateTimelinePayload,
  validateTransitionPayload,
  verifyTimeline,
  versionEtag,
} from './lib/domain.mjs';
import { IncidentStore } from './lib/store.mjs';

const MODULE_DIR = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_DATA_DIR = path.join(MODULE_DIR, 'runtime');
const MAX_BODY_BYTES = 1_048_576;
const IDENTITY_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:@-]{0,127}$/;
const IDEMPOTENCY_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const CORRELATION_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;

const STATIC_FILES = Object.freeze({
  '/': Object.freeze({
    file: path.join(MODULE_DIR, 'public', 'index.html'),
    contentType: 'text/html; charset=utf-8',
  }),
  '/index.html': Object.freeze({
    file: path.join(MODULE_DIR, 'public', 'index.html'),
    contentType: 'text/html; charset=utf-8',
  }),
  '/app.js': Object.freeze({
    file: path.join(MODULE_DIR, 'public', 'app.js'),
    contentType: 'text/javascript; charset=utf-8',
  }),
  '/styles.css': Object.freeze({
    file: path.join(MODULE_DIR, 'public', 'styles.css'),
    contentType: 'text/css; charset=utf-8',
  }),
});

export async function createIncidentCommandServer({
  dataDir = DEFAULT_DATA_DIR,
  clock = () => new Date(),
  idFactory = randomUUID,
  logger = console,
} = {}) {
  const store = await new IncidentStore({
    dataDir,
    clock,
    idFactory,
  }).init();
  const assets = await loadStaticAssets();
  const broker = new TenantEventBroker();

  const server = createServer((request, response) => {
    handleRequest({
      request,
      response,
      store,
      broker,
      assets,
      logger,
    }).catch((error) => {
      if (response.headersSent) {
        response.destroy(error);
        return;
      }
      const correlationId = correlationIdFor(request);
      sendError(response, error, correlationId, logger);
    });
  });

  server.on('clientError', (_error, socket) => {
    if (socket.writable) {
      socket.end(
        'HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n',
      );
    }
  });

  return {
    server,
    store,
    broker,
    async listen({ port = 0, host = '127.0.0.1' } = {}) {
      await new Promise((resolve, reject) => {
        const onError = (error) => {
          server.off('listening', onListening);
          reject(error);
        };
        const onListening = () => {
          server.off('error', onError);
          resolve();
        };
        server.once('error', onError);
        server.once('listening', onListening);
        server.listen(port, host);
      });
      return server.address();
    },
    async close() {
      broker.closeAll();
      if (!server.listening) {
        return;
      }
      await new Promise((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
        server.closeAllConnections?.();
      });
    },
  };
}

async function loadStaticAssets() {
  const entries = await Promise.all(
    Object.entries(STATIC_FILES).map(async ([route, descriptor]) => [
      route,
      {
        content: await readFile(descriptor.file),
        contentType: descriptor.contentType,
      },
    ]),
  );
  return Object.fromEntries(entries);
}

async function handleRequest({
  request,
  response,
  store,
  broker,
  assets,
  logger,
}) {
  const correlationId = correlationIdFor(request);
  const url = new URL(request.url ?? '/', 'http://incident-command.local');
  setCommonHeaders(response, correlationId);

  try {
    if (request.method === 'GET' && url.pathname === '/health') {
      sendJson(
        response,
        200,
        {
          status: 'ok',
          service: 'incident-command',
          persistence: 'ready',
          sseConnections: broker.size,
          timestamp: new Date(store.now()).toISOString(),
        },
        correlationId,
      );
      return;
    }

    if (
      (request.method === 'GET' || request.method === 'HEAD') &&
      Object.hasOwn(assets, url.pathname)
    ) {
      const asset = assets[url.pathname];
      response.writeHead(200, {
        'Content-Type': asset.contentType,
        'Content-Length': asset.content.length,
        'Cache-Control': 'no-store',
      });
      response.end(request.method === 'HEAD' ? undefined : asset.content);
      return;
    }

    if (!url.pathname.startsWith('/api/')) {
      throw new AppError(404, 'NOT_FOUND', 'Resource not found.');
    }

    const auth = authenticate(request);
    if (request.method === 'GET' && url.pathname === '/api/events') {
      broker.connect({ request, response, auth, correlationId });
      return;
    }

    const outcome = await routeApi({
      request,
      url,
      auth,
      store,
      broker,
    });
    sendOutcome(response, outcome, correlationId);
  } catch (error) {
    sendError(response, error, correlationId, logger);
  }
}

async function routeApi({ request, url, auth, store, broker }) {
  if (url.pathname === '/api/incidents') {
    if (request.method === 'GET') {
      const severity = optionalSingleQuery(url, 'severity');
      const state = optionalSingleQuery(url, 'state');
      if (severity && !SEVERITIES.includes(severity)) {
        throw new AppError(
          400,
          'INVALID_FILTER',
          `severity must be one of: ${SEVERITIES.join(', ')}.`,
        );
      }
      if (state && !STATES.includes(state)) {
        throw new AppError(
          400,
          'INVALID_FILTER',
          `state must be one of: ${STATES.join(', ')}.`,
        );
      }

      const now = store.now();
      const incidents = store
        .listIncidents(auth.tenantId)
        .filter(
          (incident) =>
            (!severity || incident.severity === severity) &&
            (!state || incident.state === state),
        )
        .sort(
          (left, right) =>
            Date.parse(right.createdAt) - Date.parse(left.createdAt) ||
            left.id.localeCompare(right.id),
        )
        .map((incident) => presentIncident(incident, now));
      return {
        status: 200,
        body: {
          incidents,
          count: incidents.length,
          filters: { severity: severity ?? null, state: state ?? null },
          serverTime: new Date(now).toISOString(),
        },
      };
    }

    if (request.method === 'POST') {
      authorize(auth, ['reporter']);
      const rawPayload = await readJsonBody(request);
      const payload = validateCreatePayload(rawPayload);
      return executeMutation({
        request,
        url,
        auth,
        store,
        broker,
        fingerprintBody: rawPayload,
        operation: ({ tenant, now, idFactory }) => {
          const incident = createIncidentRecord({
            tenantId: auth.tenantId,
            actorId: auth.actorId,
            actorRole: auth.role,
            payload,
            now,
            idFactory,
          });
          if (Object.hasOwn(tenant.incidents, incident.id)) {
            throw new AppError(
              409,
              'INCIDENT_ID_CONFLICT',
              'Generated incident ID already exists.',
            );
          }
          tenant.incidents[incident.id] = incident;
          const view = presentIncident(incident, now);
          return {
            status: 201,
            headers: {
              ETag: versionEtag(incident.version),
              Location: `/api/incidents/${incident.id}`,
            },
            body: { incident: view },
            events: [
              {
                type: 'incident.created',
                data: { incident: view },
              },
            ],
          };
        },
      });
    }

    throw methodNotAllowed(['GET', 'POST']);
  }

  const segments = splitPath(url.pathname);
  if (
    segments.length < 3 ||
    segments[0] !== 'api' ||
    segments[1] !== 'incidents'
  ) {
    throw new AppError(404, 'NOT_FOUND', 'API resource not found.');
  }

  const incidentId = validateIncidentId(segments[2]);
  const action = segments[3] ?? null;
  if (segments.length > 4) {
    throw new AppError(404, 'NOT_FOUND', 'API resource not found.');
  }

  if (action === null) {
    if (request.method !== 'GET') {
      throw methodNotAllowed(['GET']);
    }
    const incident = requiredIncident(store, auth.tenantId, incidentId);
    return {
      status: 200,
      headers: { ETag: versionEtag(incident.version) },
      body: { incident: presentIncident(incident, store.now()) },
    };
  }

  if (action === 'assignment') {
    if (request.method !== 'PATCH') {
      throw methodNotAllowed(['PATCH']);
    }
    authorize(auth, ['commander']);
    const expectedVersion = parseIfMatch(request);
    const rawPayload = await readJsonBody(request);
    const payload = validateAssignmentPayload(rawPayload);
    return executeMutation({
      request,
      url,
      auth,
      store,
      broker,
      fingerprintBody: rawPayload,
      expectedVersion,
      operation: ({ tenant, now }) => {
        const incident = requiredMutableIncident(tenant, incidentId);
        assertVersion(incident, expectedVersion);
        assignIncident({
          incident,
          owner: payload.owner,
          actorId: auth.actorId,
          actorRole: auth.role,
          now,
        });
        return incidentUpdatedOutcome(incident, now, 'incident.assigned');
      },
    });
  }

  if (action === 'severity') {
    if (request.method !== 'PATCH') {
      throw methodNotAllowed(['PATCH']);
    }
    authorize(auth, ['commander']);
    const expectedVersion = parseIfMatch(request);
    const rawPayload = await readJsonBody(request);
    const payload = validateSeverityPayload(rawPayload);
    return executeMutation({
      request,
      url,
      auth,
      store,
      broker,
      fingerprintBody: rawPayload,
      expectedVersion,
      operation: ({ tenant, now }) => {
        const incident = requiredMutableIncident(tenant, incidentId);
        assertVersion(incident, expectedVersion);
        changeSeverity({
          incident,
          severity: payload.severity,
          actorId: auth.actorId,
          actorRole: auth.role,
          now,
        });
        return incidentUpdatedOutcome(
          incident,
          now,
          'incident.severity_changed',
        );
      },
    });
  }

  if (action === 'timeline') {
    if (request.method !== 'POST') {
      throw methodNotAllowed(['POST']);
    }
    authorize(auth, ['responder']);
    const rawPayload = await readJsonBody(request);
    const payload = validateTimelinePayload(rawPayload);
    return executeMutation({
      request,
      url,
      auth,
      store,
      broker,
      fingerprintBody: rawPayload,
      operation: ({ tenant, now }) => {
        const incident = requiredMutableIncident(tenant, incidentId);
        const { entry } = appendTechnicalUpdate({
          incident,
          message: payload.message,
          actorId: auth.actorId,
          actorRole: auth.role,
          now,
        });
        const view = presentIncident(incident, now);
        return {
          status: 201,
          headers: { ETag: versionEtag(incident.version) },
          body: { incident: view, timelineEntry: structuredClone(entry) },
          events: [
            {
              type: 'timeline.appended',
              data: {
                incident: view,
                timelineEntry: structuredClone(entry),
              },
            },
          ],
        };
      },
    });
  }

  if (action === 'transitions') {
    if (request.method !== 'POST') {
      throw methodNotAllowed(['POST']);
    }
    authorize(auth, ['commander']);
    const expectedVersion = parseIfMatch(request);
    const rawPayload = await readJsonBody(request);
    const payload = validateTransitionPayload(rawPayload);
    return executeMutation({
      request,
      url,
      auth,
      store,
      broker,
      fingerprintBody: rawPayload,
      expectedVersion,
      operation: ({ tenant, now }) => {
        const incident = requiredMutableIncident(tenant, incidentId);
        assertVersion(incident, expectedVersion);
        transitionIncident({
          incident,
          to: payload.to,
          note: payload.note,
          actorId: auth.actorId,
          actorRole: auth.role,
          now,
        });
        return incidentUpdatedOutcome(
          incident,
          now,
          'incident.transitioned',
        );
      },
    });
  }

  if (action === 'postmortem') {
    if (request.method === 'GET') {
      const incident = requiredIncident(store, auth.tenantId, incidentId);
      const markdown = buildPostmortemMarkdown(incident);
      const filename = postmortemFilename(incident.id);
      const auditChain = verifyTimeline(
        incident.timeline,
        incident.id,
        incident.tenantId,
        incident.version,
      );
      if (url.searchParams.get('download') === '1') {
        return {
          status: 200,
          contentType: 'text/markdown; charset=utf-8',
          headers: {
            'Content-Disposition': `attachment; filename="${filename}"`,
            'X-Audit-Chain-Valid': String(auditChain.valid),
          },
          body: markdown,
        };
      }
      return {
        status: 200,
        body: {
          incidentId: incident.id,
          filename,
          markdown,
          auditChain,
        },
      };
    }

    if (request.method === 'PATCH') {
      authorize(auth, ['commander']);
      const expectedVersion = parseIfMatch(request);
      const rawPayload = await readJsonBody(request);
      const payload = validatePostmortemPayload(rawPayload);
      return executeMutation({
        request,
        url,
        auth,
        store,
        broker,
        fingerprintBody: rawPayload,
        expectedVersion,
        operation: ({ tenant, now }) => {
          const incident = requiredMutableIncident(tenant, incidentId);
          assertVersion(incident, expectedVersion);
          updatePostmortem({
            incident,
            payload,
            actorId: auth.actorId,
            actorRole: auth.role,
            now,
          });
          return incidentUpdatedOutcome(
            incident,
            now,
            'incident.postmortem_updated',
          );
        },
      });
    }

    throw methodNotAllowed(['GET', 'PATCH']);
  }

  if (action === 'audit') {
    if (request.method !== 'GET') {
      throw methodNotAllowed(['GET']);
    }
    const incident = requiredIncident(store, auth.tenantId, incidentId);
    return {
      status: 200,
      body: {
        incidentId,
        auditChain: verifyTimeline(
          incident.timeline,
          incident.id,
          incident.tenantId,
          incident.version,
        ),
      },
    };
  }

  throw new AppError(404, 'NOT_FOUND', 'API resource not found.');
}

function incidentUpdatedOutcome(incident, now, eventType) {
  const view = presentIncident(incident, now);
  return {
    status: 200,
    headers: { ETag: versionEtag(incident.version) },
    body: { incident: view },
    events: [{ type: eventType, data: { incident: view } }],
  };
}

async function executeMutation({
  request,
  url,
  auth,
  store,
  broker,
  fingerprintBody,
  expectedVersion = null,
  operation,
}) {
  const key = requireIdempotencyKey(request);
  const fingerprint = sha256(
    canonicalJson({
      actorId: auth.actorId,
      body: fingerprintBody,
      expectedVersion,
      method: request.method,
      path: url.pathname,
      role: auth.role,
    }),
  );

  const result = await store.runIdempotent({
    tenantId: auth.tenantId,
    key,
    fingerprint,
    operation,
  });
  if (result.conflict) {
    throw new AppError(
      409,
      'IDEMPOTENCY_CONFLICT',
      'Idempotency-Key was already used for a different request.',
    );
  }

  if (!result.replayed) {
    for (const event of result.events) {
      broker.publish(auth.tenantId, event.type, event.data);
    }
  }

  return {
    status: result.status,
    headers: {
      ...result.headers,
      'Idempotency-Replayed': String(result.replayed),
    },
    body: result.body,
  };
}

function authenticate(request) {
  const tenantId = singleHeader(request, 'x-tenant-id');
  const actorId = singleHeader(request, 'x-actor-id');
  const role = singleHeader(request, 'x-role');

  if (!tenantId || !actorId || !role) {
    throw new AppError(
      401,
      'AUTH_CONTEXT_REQUIRED',
      'x-tenant-id, x-actor-id, and x-role headers are required.',
    );
  }
  if (!IDENTITY_PATTERN.test(tenantId)) {
    throw new AppError(
      400,
      'INVALID_TENANT_ID',
      'x-tenant-id has an invalid format.',
    );
  }
  if (!IDENTITY_PATTERN.test(actorId)) {
    throw new AppError(
      400,
      'INVALID_ACTOR_ID',
      'x-actor-id has an invalid format.',
    );
  }
  if (!ROLES.includes(role)) {
    throw new AppError(
      403,
      'INVALID_ROLE',
      `x-role must be one of: ${ROLES.join(', ')}.`,
    );
  }
  return { tenantId, actorId, role };
}

function authorize(auth, allowedRoles) {
  if (!allowedRoles.includes(auth.role)) {
    throw new AppError(
      403,
      'FORBIDDEN',
      `Role ${auth.role} is not allowed to perform this operation.`,
      { allowedRoles },
    );
  }
}

function requireIdempotencyKey(request) {
  const key = singleHeader(request, 'idempotency-key');
  if (!key) {
    throw new AppError(
      400,
      'IDEMPOTENCY_KEY_REQUIRED',
      'Mutating operations require an Idempotency-Key header.',
    );
  }
  if (!IDEMPOTENCY_PATTERN.test(key)) {
    throw new AppError(
      400,
      'INVALID_IDEMPOTENCY_KEY',
      'Idempotency-Key has an invalid format.',
    );
  }
  return key;
}

function parseIfMatch(request) {
  const raw = singleHeader(request, 'if-match');
  if (!raw) {
    throw new AppError(
      428,
      'PRECONDITION_REQUIRED',
      'This operation requires If-Match with the current integer version.',
    );
  }
  const match = /^(?:W\/)?"?([1-9][0-9]*)"?$/.exec(raw);
  if (!match) {
    throw new AppError(
      400,
      'INVALID_IF_MATCH',
      'If-Match must contain a positive integer version.',
    );
  }
  const version = Number(match[1]);
  if (!Number.isSafeInteger(version)) {
    throw new AppError(
      400,
      'INVALID_IF_MATCH',
      'If-Match version is outside the supported integer range.',
    );
  }
  return version;
}

function assertVersion(incident, expectedVersion) {
  if (incident.version !== expectedVersion) {
    throw new AppError(
      412,
      'VERSION_CONFLICT',
      'Incident version does not match If-Match.',
      {
        expected: expectedVersion,
        actual: incident.version,
      },
    );
  }
}

function requiredIncident(store, tenantId, incidentId) {
  const incident = store.getIncident(tenantId, incidentId);
  if (!incident) {
    throw new AppError(404, 'INCIDENT_NOT_FOUND', 'Incident not found.');
  }
  return incident;
}

function requiredMutableIncident(tenant, incidentId) {
  if (!Object.hasOwn(tenant.incidents, incidentId)) {
    throw new AppError(404, 'INCIDENT_NOT_FOUND', 'Incident not found.');
  }
  return tenant.incidents[incidentId];
}

async function readJsonBody(request) {
  const contentType = singleHeader(request, 'content-type') ?? '';
  if (!/^application\/json(?:\s*;|$)/i.test(contentType)) {
    throw new AppError(
      415,
      'UNSUPPORTED_MEDIA_TYPE',
      'Content-Type must be application/json.',
    );
  }

  const declaredLength = Number(singleHeader(request, 'content-length'));
  if (Number.isFinite(declaredLength) && declaredLength > MAX_BODY_BYTES) {
    throw new AppError(
      413,
      'PAYLOAD_TOO_LARGE',
      `JSON body must not exceed ${MAX_BODY_BYTES} bytes.`,
    );
  }

  const chunks = [];
  let total = 0;
  for await (const chunk of request) {
    total += chunk.length;
    if (total > MAX_BODY_BYTES) {
      throw new AppError(
        413,
        'PAYLOAD_TOO_LARGE',
        `JSON body must not exceed ${MAX_BODY_BYTES} bytes.`,
      );
    }
    chunks.push(chunk);
  }

  if (total === 0) {
    throw new AppError(400, 'INVALID_JSON', 'JSON request body is required.');
  }

  try {
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch {
    throw new AppError(400, 'INVALID_JSON', 'Request body is not valid JSON.');
  }
}

function optionalSingleQuery(url, name) {
  const values = url.searchParams.getAll(name);
  if (values.length > 1) {
    throw new AppError(
      400,
      'INVALID_FILTER',
      `${name} may only be supplied once.`,
    );
  }
  return values.length === 0 || values[0] === '' ? null : values[0];
}

function splitPath(pathname) {
  try {
    return pathname
      .split('/')
      .filter(Boolean)
      .map((segment) => decodeURIComponent(segment));
  } catch {
    throw new AppError(400, 'INVALID_PATH', 'URL path encoding is invalid.');
  }
}

function methodNotAllowed(allowed) {
  return new AppError(
    405,
    'METHOD_NOT_ALLOWED',
    `Method not allowed. Allowed methods: ${allowed.join(', ')}.`,
    { allowed },
  );
}

function correlationIdFor(request) {
  const supplied = singleHeader(request, 'x-correlation-id');
  return supplied && CORRELATION_PATTERN.test(supplied)
    ? supplied
    : randomUUID();
}

function singleHeader(request, name) {
  const value = request.headers[name];
  if (Array.isArray(value)) {
    return value.length === 1 ? value[0].trim() : null;
  }
  return typeof value === 'string' ? value.trim() : null;
}

function setCommonHeaders(response, correlationId) {
  response.setHeader('X-Correlation-Id', correlationId);
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('Referrer-Policy', 'no-referrer');
  response.setHeader('X-Frame-Options', 'DENY');
  response.setHeader(
    'Content-Security-Policy',
    "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
  );
}

function sendOutcome(response, outcome, correlationId) {
  const headers = {
    'Cache-Control': 'no-store',
    ...(outcome.headers ?? {}),
  };
  if (outcome.contentType) {
    const body = Buffer.from(outcome.body, 'utf8');
    response.writeHead(outcome.status, {
      ...headers,
      'Content-Type': outcome.contentType,
      'Content-Length': body.length,
    });
    response.end(body);
    return;
  }
  sendJson(response, outcome.status, outcome.body, correlationId, headers);
}

function sendJson(
  response,
  status,
  body,
  _correlationId,
  extraHeaders = {},
) {
  const serialized = Buffer.from(`${JSON.stringify(body)}\n`, 'utf8');
  response.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': serialized.length,
    'Cache-Control': 'no-store',
    ...extraHeaders,
  });
  response.end(serialized);
}

function sendError(response, error, correlationId, logger) {
  if (response.headersSent || response.destroyed) {
    response.destroy();
    return;
  }

  const appError =
    error instanceof AppError
      ? error
      : new AppError(
          500,
          'INTERNAL_ERROR',
          'The server could not complete the request.',
        );
  if (!(error instanceof AppError)) {
    logger.error?.(`[${correlationId}]`, error);
  }
  sendJson(
    response,
    appError.status,
    {
      error: {
        code: appError.code,
        message: appError.message,
        correlationId,
        ...(appError.details === undefined
          ? {}
          : { details: appError.details }),
      },
    },
    correlationId,
  );
}

class TenantEventBroker {
  constructor() {
    this.clients = new Set();
    this.sequence = 0;
  }

  get size() {
    return this.clients.size;
  }

  connect({ request, response, auth, correlationId }) {
    response.writeHead(200, {
      'Content-Type': 'text/event-stream; charset=utf-8',
      'Cache-Control': 'no-cache, no-transform',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no',
    });
    response.flushHeaders?.();

    const client = {
      tenantId: auth.tenantId,
      response,
      closed: false,
      heartbeat: null,
      cleanup: null,
    };

    const cleanup = () => {
      if (client.closed) {
        return;
      }
      client.closed = true;
      clearInterval(client.heartbeat);
      this.clients.delete(client);
    };
    client.cleanup = cleanup;
    this.clients.add(client);

    safeSseWrite(
      client,
      `retry: 2000\n${formatSse({
        id: ++this.sequence,
        event: 'connected',
        data: {
          status: 'connected',
          tenantId: auth.tenantId,
          correlationId,
        },
      })}`,
    );

    client.heartbeat = setInterval(() => {
      safeSseWrite(client, `: heartbeat ${Date.now()}\n\n`);
    }, 20_000);
    client.heartbeat.unref?.();

    request.once('close', cleanup);
    response.once('close', cleanup);
    response.once('error', cleanup);
  }

  publish(tenantId, event, data) {
    const id = ++this.sequence;
    const payload = formatSse({ id, event, data });
    for (const client of this.clients) {
      if (client.tenantId === tenantId) {
        safeSseWrite(client, payload);
      }
    }
  }

  closeAll() {
    for (const client of [...this.clients]) {
      client.cleanup();
      client.response.end();
    }
  }
}

function safeSseWrite(client, payload) {
  if (client.closed || client.response.destroyed) {
    client.cleanup?.();
    return;
  }
  try {
    client.response.write(payload);
  } catch {
    client.cleanup?.();
  }
}

function formatSse({ id, event, data }) {
  return `id: ${id}\nevent: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
}

function parseCommandLine(argv) {
  const options = {
    port: Number(process.env.PORT ?? 9312),
    host: process.env.HOST ?? '127.0.0.1',
    dataDir: process.env.INCIDENT_COMMAND_DATA_DIR ?? DEFAULT_DATA_DIR,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const [name, inlineValue] = argument.split('=', 2);
    const value =
      inlineValue ??
      (() => {
        index += 1;
        return argv[index];
      })();
    if (name === '--port') {
      options.port = Number(value);
    } else if (name === '--host') {
      options.host = value;
    } else if (name === '--data-dir') {
      options.dataDir = value;
    } else {
      throw new Error(`Unknown argument: ${argument}`);
    }
  }

  if (
    !Number.isInteger(options.port) ||
    options.port < 1 ||
    options.port > 65_535
  ) {
    throw new Error('--port must be an integer from 1 through 65535.');
  }
  if (!options.host) {
    throw new Error('--host requires a value.');
  }
  if (!options.dataDir) {
    throw new Error('--data-dir requires a value.');
  }
  return options;
}

async function runFromCommandLine() {
  const options = parseCommandLine(process.argv.slice(2));
  const application = await createIncidentCommandServer({
    dataDir: options.dataDir,
  });
  await application.listen({ port: options.port, host: options.host });
  console.log(
    `Incident Command listening on http://${options.host}:${options.port}`,
  );
  console.log(`Durable state: ${application.store.filePath}`);

  let stopping = false;
  const stop = async (signal) => {
    if (stopping) {
      return;
    }
    stopping = true;
    console.log(`Received ${signal}; shutting down.`);
    await application.close();
  };
  process.once('SIGINT', () => {
    stop('SIGINT').catch((error) => {
      console.error(error);
      process.exitCode = 1;
    });
  });
  process.once('SIGTERM', () => {
    stop('SIGTERM').catch((error) => {
      console.error(error);
      process.exitCode = 1;
    });
  });
}

const invokedAsScript =
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url;

if (invokedAsScript) {
  runFromCommandLine().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
