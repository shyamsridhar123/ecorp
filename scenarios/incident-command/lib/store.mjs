import { open, mkdir, readFile, rename, rm } from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { asIso } from './domain.mjs';

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
  }
}
