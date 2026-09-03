import { createHash, randomUUID } from 'node:crypto';

export const ROLES = Object.freeze([
  'reporter',
  'responder',
  'commander',
  'auditor',
]);

export const SEVERITIES = Object.freeze(['sev1', 'sev2', 'sev3', 'sev4']);

export const STATES = Object.freeze([
  'open',
  'investigating',
  'mitigating',
  'resolved',
  'postmortem_complete',
]);

export const TRANSITIONS = Object.freeze({
  open: Object.freeze(['investigating']),
  investigating: Object.freeze(['mitigating']),
  mitigating: Object.freeze(['resolved']),
  resolved: Object.freeze(['postmortem_complete']),
  postmortem_complete: Object.freeze([]),
});

export const SLO_POLICY = Object.freeze({
  sev1: Object.freeze({ responseMinutes: 15, mitigationMinutes: 60 }),
  sev2: Object.freeze({ responseMinutes: 30, mitigationMinutes: 240 }),
  sev3: Object.freeze({ responseMinutes: 60, mitigationMinutes: 480 }),
  sev4: Object.freeze({ responseMinutes: 240, mitigationMinutes: 1_440 }),
});

export const GENESIS_HASH = '0'.repeat(64);

const INCIDENT_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export class AppError extends Error {
  constructor(status, code, message, details = undefined) {
    super(message);
    this.name = 'AppError';
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export function canonicalJson(value) {
  return JSON.stringify(canonicalize(value));
}

function canonicalize(value) {
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }

  if (value && typeof value === 'object') {
    const result = {};
    for (const key of Object.keys(value).sort()) {
      result[key] = canonicalize(value[key]);
    }
    return result;
  }

  return value;
}

export function sha256(value) {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

export function asIso(value) {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) {
    throw new Error('Clock returned an invalid date.');
  }
  return date.toISOString();
}

export function calculateDeadlines(createdAt, severity) {
  const createdMs = Date.parse(createdAt);
  const policy = SLO_POLICY[severity];
  if (!Number.isFinite(createdMs) || !policy) {
    throw new Error('Cannot calculate deadlines for invalid incident data.');
  }

  return {
    targetResponseAt: new Date(
      createdMs + policy.responseMinutes * 60_000,
    ).toISOString(),
    targetMitigationAt: new Date(
      createdMs + policy.mitigationMinutes * 60_000,
    ).toISOString(),
  };
}

export function validateIncidentId(value) {
  if (!INCIDENT_ID_PATTERN.test(value)) {
    throw new AppError(
      400,
      'INVALID_INCIDENT_ID',
      'Incident ID must be a UUID.',
    );
  }
  return value.toLowerCase();
}

export function validateCreatePayload(payload) {
  expectObject(payload);
  assertOnlyFields(payload, [
    'title',
    'severity',
    'affectedService',
    'customerImpact',
  ]);

  return {
    title: requiredString(payload.title, 'title', {
      maxLength: 160,
      singleLine: true,
    }),
    severity: severityValue(payload.severity),
    affectedService: requiredString(
      payload.affectedService,
      'affectedService',
      {
        maxLength: 120,
        singleLine: true,
      },
    ),
    customerImpact: requiredString(
      payload.customerImpact,
      'customerImpact',
      {
        maxLength: 2_000,
      },
    ),
  };
}

export function validateAssignmentPayload(payload) {
  expectObject(payload);
  assertOnlyFields(payload, ['owner']);
  return {
    owner: requiredString(payload.owner, 'owner', {
      maxLength: 128,
      singleLine: true,
      pattern: /^[A-Za-z0-9][A-Za-z0-9._:@-]{0,127}$/,
      patternMessage:
        'owner must start with an alphanumeric character and contain only identity-safe characters.',
    }),
  };
}

export function validateSeverityPayload(payload) {
  expectObject(payload);
  assertOnlyFields(payload, ['severity']);
  return { severity: severityValue(payload.severity) };
}

export function validateTimelinePayload(payload) {
  expectObject(payload);
  assertOnlyFields(payload, ['message']);
  return {
    message: requiredString(payload.message, 'message', {
      maxLength: 4_000,
    }),
  };
}

export function validateTransitionPayload(payload) {
  expectObject(payload);
  assertOnlyFields(payload, ['to', 'note']);

  if (typeof payload.to !== 'string' || !STATES.includes(payload.to)) {
    throw invalidField('to', `must be one of: ${STATES.join(', ')}.`);
  }

  return {
    to: payload.to,
    note:
      payload.note === undefined
        ? null
        : requiredString(payload.note, 'note', { maxLength: 1_000 }),
  };
}

export function validatePostmortemPayload(payload) {
  expectObject(payload);
  assertOnlyFields(payload, ['contributingFactors', 'correctiveActions']);

  if (
    payload.contributingFactors === undefined &&
    payload.correctiveActions === undefined
  ) {
    throw new AppError(
      400,
      'INVALID_INPUT',
      'At least one postmortem field is required.',
      {
        fields: {
          postmortem:
            'Provide contributingFactors, correctiveActions, or both.',
        },
      },
    );
  }

  return {
    contributingFactors:
      payload.contributingFactors === undefined
        ? undefined
        : stringList(payload.contributingFactors, 'contributingFactors'),
    correctiveActions:
      payload.correctiveActions === undefined
        ? undefined
        : stringList(payload.correctiveActions, 'correctiveActions'),
  };
}

function expectObject(payload) {
  if (
    payload === null ||
    typeof payload !== 'object' ||
    Array.isArray(payload)
  ) {
    throw new AppError(
      400,
      'INVALID_INPUT',
      'JSON request body must be an object.',
    );
  }
}

function assertOnlyFields(payload, allowedFields) {
  const unknown = Object.keys(payload).filter(
    (field) => !allowedFields.includes(field),
  );
  if (unknown.length > 0) {
    throw new AppError(
      400,
      'INVALID_INPUT',
      'Request contains unsupported fields.',
      { fields: Object.fromEntries(unknown.map((field) => [field, 'unknown'])) },
    );
  }
}

function severityValue(value) {
  if (typeof value !== 'string' || !SEVERITIES.includes(value.toLowerCase())) {
    throw invalidField(
      'severity',
      `must be one of: ${SEVERITIES.join(', ')}.`,
    );
  }
  return value.toLowerCase();
}

function requiredString(
  value,
  field,
  {
    maxLength,
    singleLine = false,
    pattern = undefined,
    patternMessage = undefined,
  },
) {
  if (typeof value !== 'string') {
    throw invalidField(field, 'must be a string.');
  }

  const normalized = value.replace(/\r\n?/g, '\n').trim();
  if (normalized.length === 0) {
    throw invalidField(field, 'must not be empty.');
  }
  if (normalized.length > maxLength) {
    throw invalidField(field, `must be at most ${maxLength} characters.`);
  }
  if (singleLine && normalized.includes('\n')) {
    throw invalidField(field, 'must be a single line.');
  }
  if (/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/u.test(normalized)) {
    throw invalidField(field, 'contains unsupported control characters.');
  }
  if (pattern && !pattern.test(normalized)) {
    throw invalidField(field, patternMessage ?? 'has an invalid format.');
  }
  return normalized;
}

function stringList(value, field) {
  if (!Array.isArray(value) || value.length > 50) {
    throw invalidField(field, 'must be an array with at most 50 items.');
  }
  return value.map((item, index) =>
    requiredString(item, `${field}[${index}]`, { maxLength: 1_000 }),
  );
}

function invalidField(field, message) {
  return new AppError(400, 'INVALID_INPUT', 'Request validation failed.', {
    fields: { [field]: message },
  });
}

export function createIncidentRecord({
  tenantId,
  actorId,
  actorRole,
  payload,
  now,
  idFactory = randomUUID,
}) {
  const timestamp = asIso(now);
  const id = idFactory().toLowerCase();
  validateIncidentId(id);
  const deadlines = calculateDeadlines(timestamp, payload.severity);

  const incident = {
    id,
    tenantId,
    title: payload.title,
    severity: payload.severity,
    affectedService: payload.affectedService,
    customerImpact: payload.customerImpact,
    owner: null,
    state: 'open',
    version: 1,
    createdAt: timestamp,
    updatedAt: timestamp,
    targetResponseAt: deadlines.targetResponseAt,
    targetMitigationAt: deadlines.targetMitigationAt,
    responseAt: null,
    mitigationAt: null,
    resolvedAt: null,
    postmortemCompletedAt: null,
    postmortem: {
      contributingFactors: [],
      correctiveActions: [],
    },
    timeline: [],
  };

  appendTimelineEntry(incident, {
    type: 'incident_created',
    message: `Incident created: ${incident.title}`,
    actorId,
    actorRole,
    timestamp,
    metadata: {
      affectedService: incident.affectedService,
      customerImpact: incident.customerImpact,
      severity: incident.severity,
      version: incident.version,
    },
  });

  return incident;
}

export function assignIncident({
  incident,
  owner,
  actorId,
  actorRole,
  now,
}) {
  if (incident.owner === owner) {
    throw new AppError(
      409,
      'NO_CHANGE',
      'The incident is already assigned to that owner.',
    );
  }

  const timestamp = asIso(now);
  const previousOwner = incident.owner;
  incident.owner = owner;
  touchIncident(incident, timestamp);
  appendTimelineEntry(incident, {
    type: 'owner_assigned',
    message: `Owner assigned to ${owner}`,
    actorId,
    actorRole,
    timestamp,
    metadata: {
      from: previousOwner,
      to: owner,
      version: incident.version,
    },
  });
  return incident;
}

export function changeSeverity({
  incident,
  severity,
  actorId,
  actorRole,
  now,
}) {
  if (incident.severity === severity) {
    throw new AppError(
      409,
      'NO_CHANGE',
      'The incident already has that severity.',
    );
  }

  const timestamp = asIso(now);
  const previousSeverity = incident.severity;
  incident.severity = severity;
  Object.assign(incident, calculateDeadlines(incident.createdAt, severity));
  touchIncident(incident, timestamp);
  appendTimelineEntry(incident, {
    type: 'severity_changed',
    message: `Severity changed from ${previousSeverity} to ${severity}`,
    actorId,
    actorRole,
    timestamp,
    metadata: {
      from: previousSeverity,
      targetMitigationAt: incident.targetMitigationAt,
      targetResponseAt: incident.targetResponseAt,
      to: severity,
      version: incident.version,
    },
  });
  return incident;
}

export function appendTechnicalUpdate({
  incident,
  message,
  actorId,
  actorRole,
  now,
}) {
  const timestamp = asIso(now);
  touchIncident(incident, timestamp);
  const entry = appendTimelineEntry(incident, {
    type: 'technical_update',
    message,
    actorId,
    actorRole,
    timestamp,
    metadata: { version: incident.version },
  });
  return { incident, entry };
}

export function transitionIncident({
  incident,
  to,
  note,
  actorId,
  actorRole,
  now,
}) {
  const from = incident.state;
  if (!TRANSITIONS[from].includes(to)) {
    throw new AppError(
      409,
      'INVALID_TRANSITION',
      `Incident cannot transition from ${from} to ${to}.`,
      { from, to, allowed: TRANSITIONS[from] },
    );
  }

  const timestamp = asIso(now);
  incident.state = to;
  if (to === 'investigating') {
    incident.responseAt = timestamp;
  } else if (to === 'mitigating') {
    incident.mitigationAt = timestamp;
  } else if (to === 'resolved') {
    incident.resolvedAt = timestamp;
  } else if (to === 'postmortem_complete') {
    incident.postmortemCompletedAt = timestamp;
  }

  touchIncident(incident, timestamp);
  const suffix = note ? ` — ${note}` : '';
  appendTimelineEntry(incident, {
    type: 'state_transition',
    message: `State changed from ${from} to ${to}${suffix}`,
    actorId,
    actorRole,
    timestamp,
    metadata: { from, note, to, version: incident.version },
  });
  return incident;
}

export function updatePostmortem({
  incident,
  payload,
  actorId,
  actorRole,
  now,
}) {
  const timestamp = asIso(now);
  const changed = [];
  if (payload.contributingFactors !== undefined) {
    incident.postmortem.contributingFactors = payload.contributingFactors;
    changed.push('contributingFactors');
  }
  if (payload.correctiveActions !== undefined) {
    incident.postmortem.correctiveActions = payload.correctiveActions;
    changed.push('correctiveActions');
  }

  touchIncident(incident, timestamp);
  appendTimelineEntry(incident, {
    type: 'postmortem_updated',
    message: `Postmortem updated: ${changed.join(', ')}`,
    actorId,
    actorRole,
    timestamp,
    metadata: { changed, version: incident.version },
  });
  return incident;
}

function touchIncident(incident, timestamp) {
  incident.version += 1;
  incident.updatedAt = timestamp;
}

export function appendTimelineEntry(
  incident,
  { type, message, actorId, actorRole, timestamp, metadata = {} },
) {
  const previousHash =
    incident.timeline.length === 0
      ? GENESIS_HASH
      : incident.timeline.at(-1).hash;
  const entry = {
    sequence: incident.timeline.length + 1,
    incidentId: incident.id,
    tenantId: incident.tenantId,
    type,
    message,
    actorId,
    actorRole,
    timestamp,
    metadata: structuredClone(metadata),
    previousHash,
  };
  entry.hash = timelineEntryHash(entry);
  incident.timeline.push(entry);
  return entry;
}

export function timelineEntryHash(entry) {
  const {
    sequence,
    incidentId,
    tenantId,
    type,
    message,
    actorId,
    actorRole,
    timestamp,
    metadata,
    previousHash,
  } = entry;
  return sha256(
    canonicalJson({
      actorId,
      actorRole,
      incidentId,
      message,
      metadata,
      previousHash,
      sequence,
      tenantId,
      timestamp,
      type,
    }),
  );
}

export function verifyTimeline(timeline) {
  const issues = [];
  let expectedPreviousHash = GENESIS_HASH;
  let expectedIncidentId = null;
  let expectedTenantId = null;

  if (!Array.isArray(timeline)) {
    return {
      valid: false,
      entries: 0,
      headHash: null,
      issues: [{ sequence: null, code: 'TIMELINE_NOT_ARRAY' }],
    };
  }

  for (let index = 0; index < timeline.length; index += 1) {
    const entry = timeline[index];
    const sequence = index + 1;
    if (!entry || typeof entry !== 'object') {
      issues.push({ sequence, code: 'ENTRY_NOT_OBJECT' });
      continue;
    }

    expectedIncidentId ??= entry.incidentId;
    expectedTenantId ??= entry.tenantId;

    if (entry.sequence !== sequence) {
      issues.push({ sequence, code: 'SEQUENCE_MISMATCH' });
    }
    if (entry.incidentId !== expectedIncidentId) {
      issues.push({ sequence, code: 'INCIDENT_MISMATCH' });
    }
    if (entry.tenantId !== expectedTenantId) {
      issues.push({ sequence, code: 'TENANT_MISMATCH' });
    }
    if (entry.previousHash !== expectedPreviousHash) {
      issues.push({ sequence, code: 'PREVIOUS_HASH_MISMATCH' });
    }

    let calculatedHash;
    try {
      calculatedHash = timelineEntryHash(entry);
    } catch {
      calculatedHash = null;
    }
    if (entry.hash !== calculatedHash) {
      issues.push({ sequence, code: 'HASH_MISMATCH' });
    }
    expectedPreviousHash =
      typeof entry.hash === 'string' ? entry.hash : expectedPreviousHash;
  }

  return {
    valid: issues.length === 0,
    entries: timeline.length,
    headHash: timeline.length > 0 ? timeline.at(-1).hash ?? null : null,
    issues,
  };
}

export function deriveSlo(incident, now) {
  const nowMs = Date.parse(asIso(now));
  const response = deriveMilestone(
    incident.targetResponseAt,
    incident.responseAt,
    nowMs,
  );
  const mitigation = deriveMilestone(
    incident.targetMitigationAt,
    incident.mitigationAt,
    nowMs,
  );

  return {
    policy: SLO_POLICY[incident.severity],
    response,
    mitigation,
    breached: response.breached || mitigation.breached,
  };
}

function deriveMilestone(deadline, metAt, nowMs) {
  const deadlineMs = Date.parse(deadline);
  if (metAt) {
    const metMs = Date.parse(metAt);
    const breached = metMs > deadlineMs;
    return {
      deadline,
      metAt,
      status: breached ? 'breached' : 'met',
      breached,
      remainingMs: 0,
      deltaMs: deadlineMs - metMs,
    };
  }

  const remainingMs = deadlineMs - nowMs;
  const breached = remainingMs < 0;
  return {
    deadline,
    metAt: null,
    status: breached ? 'breached' : 'pending',
    breached,
    remainingMs,
    deltaMs: null,
  };
}

export function presentIncident(incident, now) {
  const copy = structuredClone(incident);
  copy.slo = deriveSlo(copy, now);
  copy.auditChain = verifyTimeline(copy.timeline);
  return copy;
}

export function versionEtag(version) {
  return `"${version}"`;
}

export function buildPostmortemMarkdown(incident) {
  const audit = verifyTimeline(incident.timeline);
  const responseOutcome = completedMilestoneSummary(
    incident.targetResponseAt,
    incident.responseAt,
  );
  const mitigationOutcome = completedMilestoneSummary(
    incident.targetMitigationAt,
    incident.mitigationAt,
  );

  const lines = [
    `# Postmortem: ${markdownInline(incident.title)}`,
    '',
    `- Incident ID: \`${incident.id}\``,
    `- Severity: ${incident.severity.toUpperCase()}`,
    `- State: ${markdownInline(incident.state)}`,
    `- Affected service: ${markdownInline(incident.affectedService)}`,
    `- Owner: ${incident.owner ? markdownInline(incident.owner) : 'Unassigned'}`,
    `- Opened: ${incident.createdAt}`,
    `- Resolved: ${incident.resolvedAt ?? 'Not resolved'}`,
    `- Response target: ${incident.targetResponseAt} (${responseOutcome})`,
    `- Mitigation target: ${incident.targetMitigationAt} (${mitigationOutcome})`,
    '',
    '## Summary',
    '',
    `${markdownParagraph(incident.title)} affected ${markdownInline(
      incident.affectedService,
    )}. Current workflow state: ${markdownInline(incident.state)}.`,
    '',
    '## Impact',
    '',
    markdownParagraph(incident.customerImpact),
    '',
    '## Timeline',
    '',
  ];

  if (incident.timeline.length === 0) {
    lines.push('- No timeline entries recorded.');
  } else {
    for (const entry of incident.timeline) {
      lines.push(
        `${entry.sequence}. ${entry.timestamp} — **${markdownInline(
          entry.type,
        )}** — ${markdownParagraph(entry.message)} _(actor: ${markdownInline(
          entry.actorId,
        )}, role: ${markdownInline(entry.actorRole)})_`,
      );
    }
  }

  lines.push(
    '',
    '## Contributing factors',
    '',
    ...markdownList(
      incident.postmortem.contributingFactors,
      'None recorded.',
    ),
    '',
    '## Corrective actions',
    '',
    ...markdownList(incident.postmortem.correctiveActions, 'None recorded.'),
    '',
    '## Audit-chain verification',
    '',
    `- Status: ${audit.valid ? 'VALID' : 'INVALID'}`,
    `- Entries verified: ${audit.entries}`,
    `- Head SHA-256: ${audit.headHash ? `\`${audit.headHash}\`` : 'None'}`,
  );

  if (!audit.valid) {
    lines.push(
      `- Issues: ${audit.issues
        .map((issue) => `${issue.sequence ?? 'n/a'}:${issue.code}`)
        .join(', ')}`,
    );
  }

  return `${lines.join('\n')}\n`;
}

function completedMilestoneSummary(deadline, metAt) {
  if (!metAt) {
    return 'not recorded';
  }
  return Date.parse(metAt) <= Date.parse(deadline)
    ? `met at ${metAt}`
    : `breached; met at ${metAt}`;
}

function markdownList(items, emptyText) {
  if (items.length === 0) {
    return [`- ${emptyText}`];
  }
  return items.map((item) => `- ${markdownParagraph(item)}`);
}

function markdownInline(value) {
  return String(value)
    .replace(/\\/g, '\\\\')
    .replace(/([`*_[\]<>#|])/g, '\\$1')
    .replace(/\r\n?/g, '\n')
    .replace(/\n/g, ' ');
}

function markdownParagraph(value) {
  return markdownInline(value).replace(/\s+/g, ' ').trim();
}

export function postmortemFilename(incidentId) {
  const safeId = validateIncidentId(incidentId);
  return `incident-${safeId}-postmortem.md`;
}
