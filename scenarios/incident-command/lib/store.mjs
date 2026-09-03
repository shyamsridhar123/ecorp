import { open, mkdir, readFile, rename, rm } from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import {
  ROLES,
  SEVERITIES,
  STATES,
  asIso,
  validateIncidentId,
  verifyTimeline,
} from './domain.mjs';

const SCHEMA_VERSION = 1;

function emptyState() {
  return {
    schemaVersion: SCHEMA_VERSION,
    tenants: {},
  };
}

export class IncidentStore {
  constructor({
    dataDir,
    clock = () => new Date(),
    idFactory = randomUUID,
  }) {
    this.dataDir = path.resolve(dataDir);
    this.filePath = path.join(this.dataDir, 'state.json');
    this.clock = clock;
    this.idFactory = idFactory;
    this.state = emptyState();
    this.initialized = false;
    this.mutationQueue = Promise.resolve();
  }

  now() {
    return this.clock();
  }

  async init() {
    await mkdir(this.dataDir, { recursive: true });
    try {
      const raw = await readFile(this.filePath, 'utf8');
      const parsed = JSON.parse(raw);
      validateState(parsed);
      this.state = parsed;
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw new Error(
          `Unable to load durable incident state at ${this.filePath}: ${error.message}`,
          { cause: error },
        );
      }
    }
    this.initialized = true;
    return this;
  }

  listIncidents(tenantId) {
    this.#assertInitialized();
    const tenant = getTenant(this.state, tenantId, false);
    if (!tenant) {
      return [];
    }
    return Object.values(tenant.incidents).map((incident) =>
      structuredClone(incident),
    );
  }

  getIncident(tenantId, incidentId) {
    this.#assertInitialized();
    const tenant = getTenant(this.state, tenantId, false);
    if (!tenant || !Object.hasOwn(tenant.incidents, incidentId)) {
      return null;
    }
    return structuredClone(tenant.incidents[incidentId]);
  }

  async runIdempotent({
    tenantId,
    key,
    fingerprint,
    operation,
  }) {
    this.#assertInitialized();

    const run = async () => {
      const currentTenant = getTenant(this.state, tenantId, false);
      if (
        currentTenant &&
        Object.hasOwn(currentTenant.idempotency, key)
      ) {
        const existing = currentTenant.idempotency[key];
        if (existing.fingerprint !== fingerprint) {
          return {
            conflict: true,
            replayed: false,
            existingFingerprint: existing.fingerprint,
          };
        }
        return {
          ...structuredClone(existing.response),
          conflict: false,
          replayed: true,
          events: [],
        };
      }

      const draft = structuredClone(this.state);
      const tenant = getTenant(draft, tenantId, true);
      const result = await operation({
        tenant,
        now: this.now(),
        idFactory: this.idFactory,
      });

      const storedResponse = {
        status: result.status,
        headers: structuredClone(result.headers ?? {}),
        body: structuredClone(result.body),
      };
      tenant.idempotency[key] = {
        fingerprint,
        createdAt: asIso(this.now()),
        response: storedResponse,
      };

      await this.#persist(draft);
      this.state = draft;
      return {
        ...structuredClone(storedResponse),
        conflict: false,
        replayed: false,
        events: structuredClone(result.events ?? []),
      };
    };

    const scheduled = this.mutationQueue.then(run, run);
    this.mutationQueue = scheduled.catch(() => undefined);
    return scheduled;
  }

  async #persist(state) {
    const serialized = `${JSON.stringify(state, null, 2)}\n`;
    const temporaryPath = `${this.filePath}.${process.pid}.${randomUUID()}.tmp`;
    let handle;

    try {
      handle = await open(temporaryPath, 'wx', 0o600);
      await handle.writeFile(serialized, 'utf8');
      await handle.sync();
      await handle.close();
      handle = undefined;
      await rename(temporaryPath, this.filePath);
    } catch (error) {
      if (handle) {
        await handle.close().catch(() => undefined);
      }
      await rm(temporaryPath, { force: true }).catch(() => undefined);
      throw error;
    }
  }

  #assertInitialized() {
    if (!this.initialized) {
      throw new Error('IncidentStore.init() must complete before use.');
    }
  }
}

function getTenant(state, tenantId, create) {
  if (!Object.hasOwn(state.tenants, tenantId)) {
    if (!create) {
      return null;
    }
    state.tenants[tenantId] = {
      incidents: {},
      idempotency: {},
    };
  }
  return state.tenants[tenantId];
}

function validateState(state) {
  if (
    !state ||
    typeof state !== 'object' ||
    state.schemaVersion !== SCHEMA_VERSION ||
    !state.tenants ||
    typeof state.tenants !== 'object' ||
    Array.isArray(state.tenants)
  ) {
    throw new Error('State file has an unsupported or invalid schema.');
  }

  for (const [tenantId, tenant] of Object.entries(state.tenants)) {
    if (
      !tenant ||
      typeof tenant !== 'object' ||
      !tenant.incidents ||
      typeof tenant.incidents !== 'object' ||
      Array.isArray(tenant.incidents) ||
      !tenant.idempotency ||
      typeof tenant.idempotency !== 'object' ||
      Array.isArray(tenant.idempotency)
    ) {
      throw new Error(`State for tenant ${tenantId} is invalid.`);
    }

    for (const [incidentId, incident] of Object.entries(tenant.incidents)) {
      validatePersistedIncident(tenantId, incidentId, incident);
    }
    for (const [key, record] of Object.entries(tenant.idempotency)) {
      validatePersistedIdempotency(tenantId, key, record);
    }
  }
}

function validatePersistedIncident(tenantId, incidentId, incident) {
  if (!isRecord(incident)) {
    throw new Error(
      `Persisted incident ${incidentId} for tenant ${tenantId} is invalid.`,
    );
  }

  let normalizedIncidentId;
  try {
    normalizedIncidentId = validateIncidentId(incidentId);
  } catch {
    throw new Error(
      `Persisted incident key ${incidentId} for tenant ${tenantId} is invalid.`,
    );
  }
  if (
    normalizedIncidentId !== incidentId ||
    incident.id !== incidentId ||
    incident.tenantId !== tenantId
  ) {
    throw new Error(
      `Persisted incident ${incidentId} does not match its tenant or record key.`,
    );
  }

  for (const field of [
    'title',
    'affectedService',
    'customerImpact',
    'createdAt',
    'updatedAt',
    'targetResponseAt',
    'targetMitigationAt',
  ]) {
    requireStoredString(incident[field], `incident ${incidentId}.${field}`);
  }
  if (!SEVERITIES.includes(incident.severity)) {
    throw new Error(`Persisted incident ${incidentId} has invalid severity.`);
  }
  if (!STATES.includes(incident.state)) {
    throw new Error(`Persisted incident ${incidentId} has invalid state.`);
  }
  if (!Number.isInteger(incident.version) || incident.version < 1) {
    throw new Error(`Persisted incident ${incidentId} has invalid version.`);
  }
  if (
    incident.owner !== null &&
    (typeof incident.owner !== 'string' || incident.owner.length === 0)
  ) {
    throw new Error(`Persisted incident ${incidentId} has invalid owner.`);
  }

  for (const field of [
    'createdAt',
    'updatedAt',
    'targetResponseAt',
    'targetMitigationAt',
  ]) {
    requireStoredTimestamp(incident[field], `incident ${incidentId}.${field}`);
  }
  for (const field of [
    'responseAt',
    'mitigationAt',
    'resolvedAt',
    'postmortemCompletedAt',
  ]) {
    if (incident[field] !== null) {
      requireStoredTimestamp(incident[field], `incident ${incidentId}.${field}`);
    }
  }

  if (
    !isRecord(incident.postmortem) ||
    !isStringArray(incident.postmortem.contributingFactors) ||
    !isStringArray(incident.postmortem.correctiveActions)
  ) {
    throw new Error(`Persisted incident ${incidentId} has invalid postmortem.`);
  }
  if (!Array.isArray(incident.timeline)) {
    throw new Error(`Persisted incident ${incidentId} has invalid timeline.`);
  }
  for (const [index, entry] of incident.timeline.entries()) {
    validatePersistedTimelineEntry(incidentId, index + 1, entry);
  }

  const audit = verifyTimeline(
    incident.timeline,
    incident.id,
    incident.tenantId,
    incident.version,
  );
  if (!audit.valid) {
    const codes = audit.issues.map((issue) => issue.code).join(', ');
    throw new Error(
      `Persisted incident ${incidentId} has an invalid audit chain: ${codes}.`,
    );
  }
}

function validatePersistedTimelineEntry(incidentId, sequence, entry) {
  if (!isRecord(entry)) {
    throw new Error(
      `Persisted incident ${incidentId} has an invalid timeline entry at ${sequence}.`,
    );
  }
  for (const field of [
    'incidentId',
    'tenantId',
    'type',
    'message',
    'actorId',
    'actorRole',
    'timestamp',
    'previousHash',
    'hash',
  ]) {
    requireStoredString(
      entry[field],
      `incident ${incidentId}.timeline[${sequence}].${field}`,
    );
  }
  if (!Number.isInteger(entry.sequence) || entry.sequence < 1) {
    throw new Error(
      `Persisted incident ${incidentId} has an invalid timeline sequence.`,
    );
  }
  if (!ROLES.includes(entry.actorRole)) {
    throw new Error(
      `Persisted incident ${incidentId} has an invalid timeline actor role.`,
    );
  }
  requireStoredTimestamp(
    entry.timestamp,
    `incident ${incidentId}.timeline[${sequence}].timestamp`,
  );
  if (!isRecord(entry.metadata)) {
    throw new Error(
      `Persisted incident ${incidentId} has invalid timeline metadata.`,
    );
  }
  if (
    !/^[0-9a-f]{64}$/i.test(entry.previousHash) ||
    !/^[0-9a-f]{64}$/i.test(entry.hash)
  ) {
    throw new Error(
      `Persisted incident ${incidentId} has an invalid timeline hash.`,
    );
  }
}

function validatePersistedIdempotency(tenantId, key, record) {
  if (!key || !isRecord(record)) {
    throw new Error(
      `Persisted idempotency record ${key || '<empty>'} for tenant ${tenantId} is invalid.`,
    );
  }
  requireStoredString(
    record.fingerprint,
    `idempotency ${tenantId}/${key}.fingerprint`,
  );
  requireStoredTimestamp(
    record.createdAt,
    `idempotency ${tenantId}/${key}.createdAt`,
  );
  if (
    !isRecord(record.response) ||
    !Number.isInteger(record.response.status) ||
    record.response.status < 100 ||
    record.response.status > 599 ||
    !isRecord(record.response.headers) ||
    !Object.hasOwn(record.response, 'body')
  ) {
    throw new Error(
      `Persisted idempotency response ${key} for tenant ${tenantId} is invalid.`,
    );
  }
  for (const value of Object.values(record.response.headers)) {
    if (typeof value !== 'string') {
      throw new Error(
        `Persisted idempotency headers ${key} for tenant ${tenantId} are invalid.`,
      );
    }
  }
}

function isRecord(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function isStringArray(value) {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

function requireStoredString(value, field) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`Persisted field ${field} must be a non-empty string.`);
  }
}

function requireStoredTimestamp(value, field) {
  requireStoredString(value, field);
  if (!Number.isFinite(Date.parse(value))) {
    throw new Error(`Persisted field ${field} must be a valid timestamp.`);
  }
}
