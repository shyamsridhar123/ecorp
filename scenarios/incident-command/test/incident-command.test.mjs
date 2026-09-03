import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { createIncidentCommandServer } from '../server.mjs';
import { verifyTimeline } from '../lib/domain.mjs';

const reporter = {
  tenantId: 'tenant-alpha',
  actorId: 'reporter-one',
  role: 'reporter',
};
const responder = {
  tenantId: 'tenant-alpha',
  actorId: 'responder-one',
  role: 'responder',
};
const commander = {
  tenantId: 'tenant-alpha',
  actorId: 'commander-one',
  role: 'commander',
};
const auditor = {
  tenantId: 'tenant-alpha',
  actorId: 'auditor-one',
  role: 'auditor',
};

test('health endpoint and browser command center are launchable', async () => {
  await withServer({}, async (harness) => {
    const health = await fetch(`${harness.baseUrl}/health`);
    assert.equal(health.status, 200);
    assert.match(health.headers.get('content-type'), /application\/json/);
    assert.deepEqual((await health.json()).status, 'ok');

    const page = await fetch(`${harness.baseUrl}/`);
    assert.equal(page.status, 200);
    const html = await page.text();
    assert.match(html, /Incident Command/);
    assert.match(html, /Not a production pager replacement/);
    assert.match(html, /\/app\.js/);
  });
});

test('tenant isolation hides incidents across every read path', async () => {
  await withServer({}, async (harness) => {
    const created = await createIncident(harness, reporter);
    const incidentId = created.body.incident.id;
    const otherTenant = {
      tenantId: 'tenant-bravo',
      actorId: 'auditor-two',
      role: 'auditor',
    };

    const ownList = await api(harness, '/api/incidents', { auth: auditor });
    assert.equal(ownList.status, 200);
    assert.equal(ownList.body.count, 1);

    const isolatedList = await api(harness, '/api/incidents', {
      auth: otherTenant,
    });
    assert.equal(isolatedList.status, 200);
    assert.equal(isolatedList.body.count, 0);

    const isolatedGet = await api(
      harness,
      `/api/incidents/${incidentId}`,
      { auth: otherTenant },
    );
    assert.equal(isolatedGet.status, 404);
    assert.equal(isolatedGet.body.error.code, 'INCIDENT_NOT_FOUND');
  });
});

test('RBAC enforces reporter, responder, commander, and auditor least privilege', async () => {
  await withServer({}, async (harness) => {
    const created = await createIncident(harness, reporter);
    const incident = created.body.incident;

    const auditorCreate = await api(harness, '/api/incidents', {
      auth: auditor,
      method: 'POST',
      body: incidentPayload({ title: 'Auditor must not create' }),
    });
    assert.equal(auditorCreate.status, 403);
    assert.equal(auditorCreate.body.error.code, 'FORBIDDEN');

    const responderAssign = await api(
      harness,
      `/api/incidents/${incident.id}/assignment`,
      {
        auth: responder,
        method: 'PATCH',
        version: incident.version,
        body: { owner: 'responder-one' },
      },
    );
    assert.equal(responderAssign.status, 403);
    assert.equal(responderAssign.body.error.code, 'FORBIDDEN');

    const commanderTechnicalUpdate = await api(
      harness,
      `/api/incidents/${incident.id}/timeline`,
      {
        auth: commander,
        method: 'POST',
        body: { message: 'Commanders cannot impersonate responders.' },
      },
    );
    assert.equal(commanderTechnicalUpdate.status, 403);
    assert.equal(commanderTechnicalUpdate.body.error.code, 'FORBIDDEN');

    const auditorRead = await api(
      harness,
      `/api/incidents/${incident.id}`,
      { auth: auditor },
    );
    assert.equal(auditorRead.status, 200);
  });
});

test('invalid workflow transitions are explicitly guarded', async () => {
  await withServer({}, async (harness) => {
    const created = await createIncident(harness, reporter);
    const incident = created.body.incident;
    const transition = await api(
      harness,
      `/api/incidents/${incident.id}/transitions`,
      {
        auth: commander,
        method: 'POST',
        version: incident.version,
        body: { to: 'mitigating' },
      },
    );

    assert.equal(transition.status, 409);
    assert.equal(transition.body.error.code, 'INVALID_TRANSITION');
    assert.deepEqual(transition.body.error.details.allowed, ['investigating']);
  });
});

test('idempotency replays exact responses and rejects payload conflicts', async () => {
  await withServer({}, async (harness) => {
    const key = 'create-idempotency-test';
    const body = incidentPayload({ title: 'Payment API unavailable' });
    const first = await api(harness, '/api/incidents', {
      auth: reporter,
      method: 'POST',
      key,
      body,
    });
    const replay = await api(harness, '/api/incidents', {
      auth: reporter,
      method: 'POST',
      key,
      body: { ...body },
    });
    const conflict = await api(harness, '/api/incidents', {
      auth: reporter,
      method: 'POST',
      key,
      body: { ...body, title: 'A different incident' },
    });

    assert.equal(first.status, 201);
    assert.equal(first.headers.get('idempotency-replayed'), 'false');
    assert.equal(replay.status, 201);
    assert.equal(replay.headers.get('idempotency-replayed'), 'true');
    assert.deepEqual(replay.body, first.body);
    assert.equal(conflict.status, 409);
    assert.equal(conflict.body.error.code, 'IDEMPOTENCY_CONFLICT');

    const list = await api(harness, '/api/incidents', { auth: auditor });
    assert.equal(list.body.count, 1);
  });
});

test('optimistic concurrency requires If-Match and rejects stale versions', async () => {
  await withServer({}, async (harness) => {
    const created = await createIncident(harness, reporter);
    const incident = created.body.incident;

    const assigned = await api(
      harness,
      `/api/incidents/${incident.id}/assignment`,
      {
        auth: commander,
        method: 'PATCH',
        version: 1,
        body: { owner: 'oncall-primary' },
      },
    );
    assert.equal(assigned.status, 200);
    assert.equal(assigned.body.incident.version, 2);

    const stale = await api(
      harness,
      `/api/incidents/${incident.id}/severity`,
      {
        auth: commander,
        method: 'PATCH',
        version: 1,
        body: { severity: 'sev1' },
      },
    );
    assert.equal(stale.status, 412);
    assert.equal(stale.body.error.code, 'VERSION_CONFLICT');
    assert.deepEqual(stale.body.error.details, { expected: 1, actual: 2 });

    const missing = await api(
      harness,
      `/api/incidents/${incident.id}/severity`,
      {
        auth: commander,
        method: 'PATCH',
        body: { severity: 'sev1' },
        omitVersion: true,
      },
    );
    assert.equal(missing.status, 428);
    assert.equal(missing.body.error.code, 'PRECONDITION_REQUIRED');
  });
});

test('server-derived deadlines and breach calculations use the severity policy', async () => {
  const manual = manualClock('2026-09-01T00:00:00.000Z');
  await withServer({ clock: manual.clock }, async (harness) => {
    const created = await createIncident(harness, reporter, {
      severity: 'sev1',
    });
    const incident = created.body.incident;
    assert.equal(incident.targetResponseAt, '2026-09-01T00:15:00.000Z');
    assert.equal(incident.targetMitigationAt, '2026-09-01T01:00:00.000Z');
    assert.equal(incident.slo.response.status, 'pending');
    assert.equal(incident.slo.response.remainingMs, 15 * 60_000);

    manual.advance(16 * 60_000);
    const late = await api(
      harness,
      `/api/incidents/${incident.id}`,
      { auth: auditor },
    );
    assert.equal(late.body.incident.slo.response.status, 'breached');
    assert.equal(late.body.incident.slo.response.remainingMs, -60_000);
    assert.equal(late.body.incident.slo.mitigation.status, 'pending');

    const acknowledged = await api(
      harness,
      `/api/incidents/${incident.id}/transitions`,
      {
        auth: commander,
        method: 'POST',
        version: 1,
        body: { to: 'investigating' },
      },
    );
    assert.equal(
      acknowledged.body.incident.responseAt,
      '2026-09-01T00:16:00.000Z',
    );
    assert.equal(acknowledged.body.incident.slo.response.status, 'breached');
  });
});

test('SHA-256 timeline verification detects tampering', async () => {
  await withServer({}, async (harness) => {
    const created = await createIncident(harness, reporter);
    const incident = created.body.incident;
    const updated = await api(
      harness,
      `/api/incidents/${incident.id}/timeline`,
      {
        auth: responder,
        method: 'POST',
        body: { message: 'Traces show database connection exhaustion.' },
      },
    );

    assert.equal(updated.status, 201);
    assert.equal(updated.body.incident.auditChain.valid, true);
    assert.equal(updated.body.incident.auditChain.entries, 2);

    const tampered = structuredClone(updated.body.incident.timeline);
    tampered[0].message = 'Tampered message';
    const verification = verifyTimeline(tampered);
    assert.equal(verification.valid, false);
    assert.ok(
      verification.issues.some((issue) => issue.code === 'HASH_MISMATCH'),
    );
  });
});

test('postmortem Markdown is deterministic and contains required evidence sections', async () => {
  const manual = manualClock('2026-09-01T02:00:00.000Z');
  await withServer({ clock: manual.clock }, async (harness) => {
    let incident = (await createIncident(harness, reporter)).body.incident;

    manual.advance(5 * 60_000);
    incident = (
      await api(harness, `/api/incidents/${incident.id}/timeline`, {
        auth: responder,
        method: 'POST',
        body: { message: 'Disabled the failing dependency pool.' },
      })
    ).body.incident;

    for (const to of ['investigating', 'mitigating', 'resolved']) {
      manual.advance(60_000);
      incident = (
        await api(harness, `/api/incidents/${incident.id}/transitions`, {
          auth: commander,
          method: 'POST',
          version: incident.version,
          body: { to },
        })
      ).body.incident;
    }

    incident = (
      await api(harness, `/api/incidents/${incident.id}/postmortem`, {
        auth: commander,
        method: 'PATCH',
        version: incident.version,
        body: {
          contributingFactors: ['Connection pool limits were undersized.'],
          correctiveActions: ['Add saturation alerts and load tests.'],
        },
      })
    ).body.incident;
    incident = (
      await api(harness, `/api/incidents/${incident.id}/transitions`, {
        auth: commander,
        method: 'POST',
        version: incident.version,
        body: { to: 'postmortem_complete' },
      })
    ).body.incident;

    const first = await api(
      harness,
      `/api/incidents/${incident.id}/postmortem`,
      { auth: auditor },
    );
    const second = await api(
      harness,
      `/api/incidents/${incident.id}/postmortem`,
      { auth: auditor },
    );
    assert.equal(first.body.markdown, second.body.markdown);
    assert.match(first.body.markdown, /^# Postmortem:/);
    assert.match(first.body.markdown, /## Summary/);
    assert.match(first.body.markdown, /## Impact/);
    assert.match(first.body.markdown, /## Timeline/);
    assert.match(first.body.markdown, /## Contributing factors/);
    assert.match(first.body.markdown, /Connection pool limits were undersized/);
    assert.match(first.body.markdown, /## Corrective actions/);
    assert.match(first.body.markdown, /Add saturation alerts and load tests/);
    assert.match(first.body.markdown, /## Audit-chain verification/);
    assert.match(first.body.markdown, /Status: VALID/);
    assert.equal(first.body.auditChain.valid, true);

    const download = await api(
      harness,
      `/api/incidents/${incident.id}/postmortem?download=1`,
      { auth: auditor },
    );
    assert.equal(download.status, 200);
    assert.match(
      download.headers.get('content-disposition'),
      /^attachment; filename="incident-[0-9a-f-]+-postmortem\.md"$/,
    );
    assert.equal(download.body, first.body.markdown);
  });
});

test('SSE emits only tenant-scoped incident changes', { timeout: 8_000 }, async () => {
  await withServer({}, async (harness) => {
    const tenantA = reporter;
    const tenantB = {
      tenantId: 'tenant-bravo',
      actorId: 'auditor-bravo',
      role: 'auditor',
    };
    const streamA = await openSse(harness, tenantA);
    const streamB = await openSse(harness, tenantB);

    try {
      assert.equal((await nextSse(streamA)).event, 'connected');
      assert.equal((await nextSse(streamB)).event, 'connected');
      const created = await createIncident(harness, tenantA);
      assert.equal(created.status, 201);

      const tenantAEvent = await nextSse(streamA);
      assert.equal(tenantAEvent.event, 'incident.created');
      assert.equal(
        tenantAEvent.data.incident.tenantId,
        tenantA.tenantId,
      );

      await assert.rejects(
        nextSse(streamB, 250),
        /Timed out waiting for SSE event/,
      );
    } finally {
      await closeSse(streamA);
      await closeSse(streamB);
    }
  });
});

test('durable state and idempotency survive a process restart', async () => {
  const dataDir = await mkdtemp(path.join(os.tmpdir(), 'incident-command-'));
  const first = await startHarness({ dataDir });
  const key = 'restart-safe-create';
  const body = incidentPayload({ title: 'Durability check' });
  let created;
  try {
    created = await api(first, '/api/incidents', {
      auth: reporter,
      method: 'POST',
      key,
      body,
    });
    assert.equal(created.status, 201);
    const persisted = JSON.parse(
      await readFile(path.join(dataDir, 'state.json'), 'utf8'),
    );
    assert.equal(persisted.schemaVersion, 1);
  } finally {
    await first.app.close();
  }

  const second = await startHarness({ dataDir });
  try {
    const fetched = await api(
      second,
      `/api/incidents/${created.body.incident.id}`,
      { auth: auditor },
    );
    assert.equal(fetched.status, 200);
    assert.equal(fetched.body.incident.title, 'Durability check');

    const replay = await api(second, '/api/incidents', {
      auth: reporter,
      method: 'POST',
      key,
      body,
    });
    assert.equal(replay.status, 201);
    assert.equal(replay.headers.get('idempotency-replayed'), 'true');
    assert.deepEqual(replay.body, created.body);
  } finally {
    await second.app.close();
    await rm(dataDir, { recursive: true, force: true });
  }
});

test('path-like incident IDs cannot expose arbitrary local files', async () => {
  await withServer({}, async (harness) => {
    const response = await api(
      harness,
      '/api/incidents/%2e%2e%2f%2e%2e%2fSPEC.md/postmortem?download=1',
      { auth: auditor },
    );
    assert.ok([400, 404].includes(response.status));
    assert.doesNotMatch(JSON.stringify(response.body), /enterprise incident response control room/);
  });
});

async function withServer(options, callback) {
  const dataDir = await mkdtemp(path.join(os.tmpdir(), 'incident-command-'));
  const harness = await startHarness({ ...options, dataDir });
  try {
    await callback(harness);
  } finally {
    await harness.app.close();
    await rm(dataDir, { recursive: true, force: true });
  }
}

async function startHarness({ dataDir, clock } = {}) {
  const app = await createIncidentCommandServer({
    dataDir,
    clock,
    logger: { error() {} },
  });
  const address = await app.listen({ port: 0, host: '127.0.0.1' });
  return {
    app,
    baseUrl: `http://127.0.0.1:${address.port}`,
  };
}

async function api(
  harness,
  route,
  {
    auth = reporter,
    method = 'GET',
    body = undefined,
    key = randomUUID(),
    version = undefined,
    omitVersion = false,
  } = {},
) {
  const headers = {
    'x-tenant-id': auth.tenantId,
    'x-actor-id': auth.actorId,
    'x-role': auth.role,
  };
  if (body !== undefined) {
    headers['content-type'] = 'application/json';
  }
  if (!['GET', 'HEAD'].includes(method)) {
    headers['idempotency-key'] = key;
  }
  if (!omitVersion && version !== undefined) {
    headers['if-match'] = `"${version}"`;
  }

  const response = await fetch(`${harness.baseUrl}${route}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const contentType = response.headers.get('content-type') ?? '';
  const parsedBody = contentType.includes('application/json')
    ? await response.json()
    : await response.text();
  return {
    status: response.status,
    headers: response.headers,
    body: parsedBody,
  };
}

async function createIncident(harness, auth = reporter, overrides = {}) {
  return api(harness, '/api/incidents', {
    auth,
    method: 'POST',
    body: incidentPayload(overrides),
  });
}

function incidentPayload(overrides = {}) {
  return {
    title: 'Checkout requests failing',
    severity: 'sev2',
    affectedService: 'checkout-api',
    customerImpact: 'Customers cannot complete purchases.',
    ...overrides,
  };
}

function manualClock(initialIso) {
  let milliseconds = Date.parse(initialIso);
  return {
    clock: () => new Date(milliseconds),
    advance(delta) {
      milliseconds += delta;
    },
  };
}

async function openSse(harness, auth) {
  const controller = new AbortController();
  const response = await fetch(`${harness.baseUrl}/api/events`, {
    headers: {
      'x-tenant-id': auth.tenantId,
      'x-actor-id': auth.actorId,
      'x-role': auth.role,
    },
    signal: controller.signal,
  });
  assert.equal(response.status, 200);
  assert.match(response.headers.get('content-type'), /text\/event-stream/);
  return {
    controller,
    reader: response.body.getReader(),
    decoder: new TextDecoder(),
    buffer: '',
    queued: [],
  };
}

async function nextSse(connection, timeout = 2_000) {
  const deadline = Date.now() + timeout;
  while (true) {
    parseAvailableSse(connection);
    if (connection.queued.length > 0) {
      return connection.queued.shift();
    }

    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      throw new Error('Timed out waiting for SSE event');
    }
    const timeoutToken = Symbol('timeout');
    const result = await Promise.race([
      connection.reader.read(),
      new Promise((resolve) => setTimeout(() => resolve(timeoutToken), remaining)),
    ]);
    if (result === timeoutToken) {
      throw new Error('Timed out waiting for SSE event');
    }
    if (result.done) {
      throw new Error('SSE stream ended before the expected event');
    }
    connection.buffer += connection.decoder.decode(result.value, {
      stream: true,
    });
  }
}

function parseAvailableSse(connection) {
  let boundary;
  while ((boundary = connection.buffer.indexOf('\n\n')) !== -1) {
    const block = connection.buffer.slice(0, boundary);
    connection.buffer = connection.buffer.slice(boundary + 2);
    const lines = block.split('\n');
    if (lines.every((line) => line.startsWith(':') || line === '')) {
      continue;
    }
    const event = { event: 'message', data: null, id: null };
    const data = [];
    for (const line of lines) {
      if (line.startsWith('event:')) {
        event.event = line.slice(6).trim();
      } else if (line.startsWith('id:')) {
        event.id = line.slice(3).trim();
      } else if (line.startsWith('data:')) {
        data.push(line.slice(5).trimStart());
      }
    }
    if (data.length > 0) {
      event.data = JSON.parse(data.join('\n'));
    }
    connection.queued.push(event);
  }
}

async function closeSse(connection) {
  connection.controller.abort();
  await connection.reader.cancel().catch(() => undefined);
}
