const state = {
  identity: {
    tenantId: 'acme-ops',
    actorId: 'alice',
    role: 'reporter',
  },
  incidents: [],
  selected: null,
  selectedIncidentId: null,
  streamController: null,
  streamGeneration: 0,
  scopeGeneration: 0,
  selectionGeneration: 0,
  listRequestGeneration: 0,
  refreshTimer: null,
  postmortemDraft: {
    incidentId: null,
    dirty: false,
  },
};

const nextState = {
  open: 'investigating',
  investigating: 'mitigating',
  mitigating: 'resolved',
  resolved: 'postmortem_complete',
  postmortem_complete: null,
};

const elements = {
  identityForm: document.querySelector('#identity-form'),
  tenantId: document.querySelector('#tenant-id'),
  actorId: document.querySelector('#actor-id'),
  role: document.querySelector('#role'),
  streamDot: document.querySelector('#stream-dot'),
  streamStatus: document.querySelector('#stream-status'),
  refreshButton: document.querySelector('#refresh-button'),
  filterSeverity: document.querySelector('#filter-severity'),
  filterState: document.querySelector('#filter-state'),
  incidentCount: document.querySelector('#incident-count'),
  incidentList: document.querySelector('#incident-list'),
  createForm: document.querySelector('#create-form'),
  createPermission: document.querySelector('#create-permission'),
  emptyState: document.querySelector('#empty-state'),
  detail: document.querySelector('#incident-detail'),
  detailId: document.querySelector('#detail-id'),
  detailTitle: document.querySelector('#detail-title'),
  detailSeverity: document.querySelector('#detail-severity'),
  detailState: document.querySelector('#detail-state'),
  detailImpact: document.querySelector('#detail-impact'),
  detailService: document.querySelector('#detail-service'),
  detailOwner: document.querySelector('#detail-owner'),
  detailVersion: document.querySelector('#detail-version'),
  detailAudit: document.querySelector('#detail-audit'),
  responseSlo: document.querySelector('#response-slo'),
  mitigationSlo: document.querySelector('#mitigation-slo'),
  assignmentForm: document.querySelector('#assignment-form'),
  severityForm: document.querySelector('#severity-form'),
  transitionForm: document.querySelector('#transition-form'),
  transitionButton: document.querySelector('#transition-button'),
  commanderLock: document.querySelector('#commander-lock'),
  timelineForm: document.querySelector('#timeline-form'),
  responderLock: document.querySelector('#responder-lock'),
  timelineHead: document.querySelector('#timeline-head'),
  timelineList: document.querySelector('#timeline-list'),
  postmortemForm: document.querySelector('#postmortem-form'),
  postmortemLock: document.querySelector('#postmortem-lock'),
  previewButton: document.querySelector('#preview-button'),
  downloadButton: document.querySelector('#download-button'),
  postmortemPreview: document.querySelector('#postmortem-preview'),
  toast: document.querySelector('#toast'),
};

class ApiError extends Error {
  constructor(response, payload) {
    const detail = payload?.error;
    super(detail?.message ?? `Request failed with status ${response.status}`);
    this.status = response.status;
    this.code = detail?.code ?? 'REQUEST_FAILED';
    this.correlationId =
      detail?.correlationId ?? response.headers.get('x-correlation-id');
    this.details = detail?.details;
  }
}

function loadSavedIdentity() {
  try {
    const saved = JSON.parse(
      localStorage.getItem('incident-command-identity') ?? 'null',
    );
    if (saved?.tenantId && saved?.actorId && saved?.role) {
      state.identity = saved;
    }
  } catch {
    // A malformed local preference should never block the control room.
  }
  elements.tenantId.value = state.identity.tenantId;
  elements.actorId.value = state.identity.actorId;
  elements.role.value = state.identity.role;
}

function identityHeaders() {
  return {
    'x-tenant-id': state.identity.tenantId,
    'x-actor-id': state.identity.actorId,
    'x-role': state.identity.role,
  };
}

async function apiRequest(
  path,
  { method = 'GET', body = undefined, version = undefined } = {},
) {
  const headers = identityHeaders();
  if (body !== undefined) {
    headers['content-type'] = 'application/json';
  }
  if (!['GET', 'HEAD'].includes(method)) {
    headers['idempotency-key'] = crypto.randomUUID();
  }
  if (version !== undefined) {
    headers['if-match'] = `"${version}"`;
  }

  const response = await fetch(path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const contentType = response.headers.get('content-type') ?? '';
  const payload = contentType.includes('application/json')
    ? await response.json()
    : await response.text();
  if (!response.ok) {
    throw new ApiError(response, payload);
  }
  return { response, payload };
}

async function loadIncidents({
  preserveSelection = true,
  generation = state.scopeGeneration,
} = {}) {
  const requestGeneration = ++state.listRequestGeneration;
  const params = new URLSearchParams();
  if (elements.filterSeverity.value) {
    params.set('severity', elements.filterSeverity.value);
  }
  if (elements.filterState.value) {
    params.set('state', elements.filterState.value);
  }
  const query = params.size ? `?${params}` : '';
  let payload;
  try {
    ({ payload } = await apiRequest(`/api/incidents${query}`));
  } catch (error) {
    if (!isCurrentListRequest(generation, requestGeneration)) {
      return false;
    }
    throw error;
  }
  if (!isCurrentListRequest(generation, requestGeneration)) {
    return false;
  }
  state.incidents = payload.incidents;
  elements.incidentCount.textContent = `${payload.count} incident${
    payload.count === 1 ? '' : 's'
  } in this tenant`;
  renderIncidentList();

  if (preserveSelection && state.selectedIncidentId) {
    const stillVisible = state.incidents.find(
      (incident) => incident.id === state.selectedIncidentId,
    );
    if (stillVisible) {
      await selectIncident(stillVisible.id, {
        quiet: true,
        preservePostmortemDraft: true,
        generation,
        selectionGeneration: state.selectionGeneration,
        listRequestGeneration: requestGeneration,
      });
    } else {
      clearSelection();
    }
  }
  return isCurrentListRequest(generation, requestGeneration);
}

function isCurrentListRequest(scopeGeneration, listRequestGeneration) {
  return (
    scopeGeneration === state.scopeGeneration &&
    listRequestGeneration === state.listRequestGeneration
  );
}

function renderIncidentList() {
  elements.incidentList.replaceChildren();
  if (state.incidents.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'list-empty';
    empty.textContent = 'No incidents match this queue.';
    elements.incidentList.append(empty);
    return;
  }

  for (const incident of state.incidents) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'incident-card';
    if (state.selectedIncidentId === incident.id) {
      button.classList.add('selected');
    }
    button.addEventListener('click', () => {
      selectIncident(incident.id).catch(showError);
    });

    const top = document.createElement('span');
    top.className = 'incident-card-top';
    const severity = document.createElement('strong');
    severity.className = `severity-text ${incident.severity}`;
    severity.textContent = incident.severity.toUpperCase();
    const workflow = document.createElement('span');
    workflow.className = 'state-text';
    workflow.textContent = humanize(incident.state);
    top.append(severity, workflow);

    const title = document.createElement('span');
    title.className = 'incident-card-title';
    title.textContent = incident.title;
    const service = document.createElement('span');
    service.className = 'incident-card-meta';
    service.textContent = `${incident.affectedService} · ${
      incident.owner ?? 'unassigned'
    }`;
    const slo = document.createElement('span');
    slo.className = incident.slo.breached
      ? 'incident-card-slo breached'
      : 'incident-card-slo';
    slo.textContent = incident.slo.breached
      ? 'SLO breached'
      : `Response ${formatSloValue(incident.slo.response)}`;
    button.append(top, title, service, slo);
    elements.incidentList.append(button);
  }
}

async function selectIncident(
  id,
  {
    quiet = false,
    preservePostmortemDraft = false,
    generation = state.scopeGeneration,
    selectionGeneration = null,
    listRequestGeneration = null,
  } = {},
) {
  let requestSelectionGeneration = selectionGeneration;
  if (requestSelectionGeneration === null) {
    state.selectionGeneration += 1;
    requestSelectionGeneration = state.selectionGeneration;
    state.selectedIncidentId = id;
    clearPostmortemPreview();
  } else if (
    requestSelectionGeneration !== state.selectionGeneration ||
    state.selectedIncidentId !== id
  ) {
    return false;
  }
  const { payload } = await apiRequest(`/api/incidents/${encodeURIComponent(id)}`);
  if (
    generation !== state.scopeGeneration ||
    (listRequestGeneration !== null &&
      listRequestGeneration !== state.listRequestGeneration) ||
    requestSelectionGeneration !== state.selectionGeneration ||
    state.selectedIncidentId !== id ||
    payload.incident.id !== id
  ) {
    return false;
  }
  state.selected = payload.incident;
  renderIncidentList();
  renderDetail({ preservePostmortemDraft });
  if (!quiet) {
    elements.detail.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
  return true;
}

function clearSelection() {
  state.selectionGeneration += 1;
  state.selected = null;
  state.selectedIncidentId = null;
  state.postmortemDraft = {
    incidentId: null,
    dirty: false,
  };
  elements.detail.classList.add('hidden');
  elements.emptyState.classList.remove('hidden');
  clearPostmortemPreview();
  renderIncidentList();
}

function clearPostmortemPreview() {
  elements.postmortemPreview.textContent = '';
  elements.postmortemPreview.classList.add('hidden');
}

function clearIncidentScope() {
  state.scopeGeneration += 1;
  clearTimeout(state.refreshTimer);
  state.refreshTimer = null;
  state.incidents = [];
  elements.incidentCount.textContent = 'Loading incidents…';
  clearSelection();
  return state.scopeGeneration;
}

function renderDetail({ preservePostmortemDraft = false } = {}) {
  const incident = state.selected;
  if (!incident) {
    clearSelection();
    return;
  }

  elements.emptyState.classList.add('hidden');
  elements.detail.classList.remove('hidden');
  elements.detailId.textContent = incident.id;
  elements.detailTitle.textContent = incident.title;
  elements.detailSeverity.textContent = incident.severity.toUpperCase();
  elements.detailSeverity.className = `badge severity-badge ${incident.severity}`;
  elements.detailState.textContent = humanize(incident.state);
  elements.detailImpact.textContent = incident.customerImpact;
  elements.detailService.textContent = incident.affectedService;
  elements.detailOwner.textContent = incident.owner ?? 'Unassigned';
  elements.detailVersion.textContent = String(incident.version);
  elements.detailAudit.textContent = incident.auditChain.valid
    ? `Valid · ${incident.auditChain.entries} entries`
    : `Invalid · ${incident.auditChain.issues.length} issues`;
  elements.detailAudit.className = incident.auditChain.valid
    ? 'audit-valid'
    : 'audit-invalid';
  elements.timelineHead.textContent = incident.auditChain.headHash
    ? `head ${incident.auditChain.headHash.slice(0, 12)}…`
    : 'empty chain';

  elements.assignmentForm.elements.owner.value = incident.owner ?? '';
  elements.severityForm.elements.severity.value = incident.severity;
  const destination = nextState[incident.state];
  elements.transitionButton.textContent = destination
    ? `Advance to ${humanize(destination)}`
    : 'Workflow complete';
  elements.transitionButton.disabled =
    state.identity.role !== 'commander' || !destination;

  renderSloCard(elements.responseSlo, incident.slo.response);
  renderSloCard(elements.mitigationSlo, incident.slo.mitigation);
  renderTimeline(incident.timeline);
  const keepDraft =
    preservePostmortemDraft &&
    state.postmortemDraft.dirty &&
    state.postmortemDraft.incidentId === incident.id;
  if (!keepDraft) {
    elements.postmortemForm.elements.contributingFactors.value =
      incident.postmortem.contributingFactors.join('\n');
    elements.postmortemForm.elements.correctiveActions.value =
      incident.postmortem.correctiveActions.join('\n');
    state.postmortemDraft = {
      incidentId: incident.id,
      dirty: false,
    };
  }
  applyPermissions();
}

function renderSloCard(container, milestone) {
  const liveRemaining = remaining(milestone);
  const liveBreach =
    milestone.breached || (!milestone.metAt && liveRemaining < 0);
  container.classList.toggle('breached', liveBreach);
  container.classList.toggle('met', milestone.status === 'met');
  container.querySelector('.slo-value').textContent = formatSloValue(milestone);
  container.querySelector('.slo-deadline').textContent = new Date(
    milestone.deadline,
  ).toLocaleString();
}

function formatSloValue(milestone) {
  if (milestone.status === 'met') {
    return 'Met';
  }
  if (milestone.metAt) {
    return 'Breached';
  }
  const liveRemaining = remaining(milestone);
  if (milestone.status === 'breached' || liveRemaining < 0) {
    return `Late ${formatDuration(-liveRemaining)}`;
  }
  return formatDuration(liveRemaining);
}

function remaining(milestone) {
  if (milestone.metAt) {
    return 0;
  }
  return Date.parse(milestone.deadline) - Date.now();
}

function formatDuration(milliseconds) {
  const absolute = Math.max(0, Math.abs(milliseconds));
  const totalSeconds = Math.floor(absolute / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}h ${String(minutes).padStart(2, '0')}m`;
  }
  return `${minutes}m ${String(seconds).padStart(2, '0')}s`;
}

function renderTimeline(entries) {
  elements.timelineList.replaceChildren();
  const indexedEntries = entries.map((entry, index) => ({ entry, index }));
  for (const { entry, index } of indexedEntries.reverse()) {
    const item = document.createElement('li');
    const marker = document.createElement('span');
    const entryIsObject =
      entry !== null && typeof entry === 'object' && !Array.isArray(entry);
    marker.className = entryIsObject
      ? `timeline-marker ${entry.type}`
      : 'timeline-marker';
    marker.setAttribute('aria-hidden', 'true');
    const content = document.createElement('div');
    const heading = document.createElement('div');
    heading.className = 'timeline-heading';
    const type = document.createElement('strong');
    const time = document.createElement('time');
    const message = document.createElement('p');
    const actor = document.createElement('small');

    if (entryIsObject) {
      type.textContent = humanize(entry.type);
      time.dateTime = entry.timestamp;
      time.textContent = new Date(entry.timestamp).toLocaleString();
      message.textContent = entry.message;
      const hash =
        typeof entry.hash === 'string' ? `${entry.hash.slice(0, 10)}…` : 'no hash';
      actor.textContent = `${entry.actorId} · ${entry.actorRole} · #${entry.sequence} · ${hash}`;
    } else {
      type.textContent = 'Invalid timeline entry';
      message.textContent = 'This audit entry is malformed.';
      actor.textContent = `#${index + 1} · ENTRY_NOT_OBJECT`;
    }

    heading.append(type, time);
    content.append(heading, message, actor);
    item.append(marker, content);
    elements.timelineList.append(item);
  }
}

function applyPermissions() {
  const role = state.identity.role;
  const reporter = role === 'reporter';
  const responder = role === 'responder';
  const commander = role === 'commander';

  setFormDisabled(elements.createForm, !reporter);
  elements.createPermission.textContent = reporter
    ? 'Reporter access active.'
    : `Current role is ${role}; switch to reporter to declare incidents.`;

  setFormDisabled(elements.assignmentForm, !commander);
  setFormDisabled(elements.severityForm, !commander);
  setFormDisabled(elements.transitionForm, !commander);
  if (state.selected && !nextState[state.selected.state]) {
    elements.transitionButton.disabled = true;
  }
  elements.commanderLock.textContent = commander
    ? 'Commander access active'
    : `Read-only for ${role}`;

  setFormDisabled(elements.timelineForm, !responder);
  elements.responderLock.textContent = responder
    ? 'Responder access active'
    : `Technical updates require responder`;

  setFormDisabled(elements.postmortemForm, !commander);
  elements.postmortemLock.textContent = commander
    ? 'Commander access active'
    : `Editing requires commander`;
}

function setFormDisabled(form, disabled) {
  for (const control of form.elements) {
    control.disabled = disabled;
  }
}

async function submitCreate(event) {
  event.preventDefault();
  const form = new FormData(elements.createForm);
  const scopeGeneration = state.scopeGeneration;
  let payload;
  try {
    ({ payload } = await apiRequest('/api/incidents', {
      method: 'POST',
      body: {
        title: form.get('title'),
        severity: form.get('severity'),
        affectedService: form.get('affectedService'),
        customerImpact: form.get('customerImpact'),
      },
    }));
  } catch (error) {
    if (scopeGeneration !== state.scopeGeneration) {
      return false;
    }
    throw error;
  }
  if (scopeGeneration !== state.scopeGeneration) {
    return false;
  }
  elements.createForm.reset();
  elements.createForm.elements.severity.value = 'sev2';
  const loaded = await loadIncidents({
    preserveSelection: false,
    generation: scopeGeneration,
  });
  if (!loaded || scopeGeneration !== state.scopeGeneration) {
    return false;
  }
  const selected = await selectIncident(payload.incident.id, {
    generation: scopeGeneration,
  });
  if (!selected) {
    return false;
  }
  showToast('Incident declared and audit chain started.');
  return true;
}

async function submitAssignment(event) {
  event.preventDefault();
  const owner = new FormData(elements.assignmentForm).get('owner');
  if (await mutateSelected('assignment', 'PATCH', { owner })) {
    showToast('Owner assignment recorded.');
  }
}

async function submitSeverity(event) {
  event.preventDefault();
  const severity = new FormData(elements.severityForm).get('severity');
  if (await mutateSelected('severity', 'PATCH', { severity })) {
    showToast('Severity and server-owned deadlines updated.');
  }
}

async function submitTransition(event) {
  event.preventDefault();
  const destination = nextState[state.selected?.state];
  if (!destination) {
    return;
  }
  const note = new FormData(elements.transitionForm).get('note').trim();
  const mutated = await mutateSelected('transitions', 'POST', {
    to: destination,
    ...(note ? { note } : {}),
  });
  if (mutated) {
    elements.transitionForm.elements.note.value = '';
    showToast(`Workflow advanced to ${humanize(destination)}.`);
  }
}

async function submitTimeline(event) {
  event.preventDefault();
  const message = new FormData(elements.timelineForm).get('message');
  if (
    await mutateSelected('timeline', 'POST', { message }, { versioned: false })
  ) {
    elements.timelineForm.reset();
    showToast('Technical update appended to the hash chain.');
  }
}

async function submitPostmortem(event) {
  event.preventDefault();
  const form = new FormData(elements.postmortemForm);
  if (
    await mutateSelected('postmortem', 'PATCH', {
      contributingFactors: lines(form.get('contributingFactors')),
      correctiveActions: lines(form.get('correctiveActions')),
    })
  ) {
    showToast('Postmortem details saved.');
  }
}

async function mutateSelected(
  action,
  method,
  body,
  { versioned = true } = {},
) {
  if (!state.selected) {
    return false;
  }
  const id = state.selected.id;
  const generation = state.scopeGeneration;
  const selectionGeneration = state.selectionGeneration;
  const version = state.selected.version;
  try {
    const { payload } = await apiRequest(
      `/api/incidents/${encodeURIComponent(id)}/${action}`,
      {
        method,
        body,
        version: versioned ? version : undefined,
      },
    );
    if (!isCurrentSelection(id, generation, selectionGeneration)) {
      return false;
    }
    state.selected = payload.incident;
    if (action === 'postmortem') {
      state.postmortemDraft = {
        incidentId: id,
        dirty: false,
      };
    }
    await loadIncidents();
    return isCurrentSelection(id, generation, selectionGeneration);
  } catch (error) {
    if (!isCurrentSelection(id, generation, selectionGeneration)) {
      return false;
    }
    if (error instanceof ApiError && error.code === 'VERSION_CONFLICT') {
      await selectIncident(id, {
        quiet: true,
        preservePostmortemDraft: action === 'postmortem',
        generation,
        selectionGeneration,
      });
      if (!isCurrentSelection(id, generation, selectionGeneration)) {
        return false;
      }
    }
    throw error;
  }
}

function isCurrentSelection(id, scopeGeneration, selectionGeneration) {
  return (
    scopeGeneration === state.scopeGeneration &&
    selectionGeneration === state.selectionGeneration &&
    state.selected?.id === id
  );
}

async function previewPostmortem() {
  if (!state.selected) {
    return;
  }
  const incidentId = state.selected.id;
  const scopeGeneration = state.scopeGeneration;
  const selectionGeneration = state.selectionGeneration;
  const { payload } = await apiRequest(
    `/api/incidents/${encodeURIComponent(incidentId)}/postmortem`,
  );
  if (
    scopeGeneration !== state.scopeGeneration ||
    selectionGeneration !== state.selectionGeneration ||
    state.selected?.id !== incidentId ||
    payload.incidentId !== incidentId
  ) {
    return false;
  }
  elements.postmortemPreview.textContent = payload.markdown;
  elements.postmortemPreview.classList.remove('hidden');
  elements.postmortemPreview.scrollIntoView({
    behavior: 'smooth',
    block: 'nearest',
  });
  return true;
}

async function downloadPostmortem() {
  if (!state.selected) {
    return false;
  }
  const incident = state.selected;
  const incidentId = incident.id;
  const scopeGeneration = state.scopeGeneration;
  const selectionGeneration = state.selectionGeneration;
  let response;
  let payload;
  try {
    ({ response, payload } = await apiRequest(
      `/api/incidents/${encodeURIComponent(incidentId)}/postmortem?download=1`,
    ));
  } catch (error) {
    if (
      !isCurrentSelection(
        incidentId,
        scopeGeneration,
        selectionGeneration,
      )
    ) {
      return false;
    }
    throw error;
  }
  if (
    !isCurrentSelection(incidentId, scopeGeneration, selectionGeneration)
  ) {
    return false;
  }
  const disposition = response.headers.get('content-disposition') ?? '';
  const match = /filename="([^"]+)"/.exec(disposition);
  const filename = match?.[1] ?? `incident-${incidentId}-postmortem.md`;
  const objectUrl = URL.createObjectURL(
    new Blob([payload], { type: 'text/markdown;charset=utf-8' }),
  );
  const anchor = document.createElement('a');
  anchor.href = objectUrl;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(objectUrl);
  showToast(`Postmortem for ${incident.title} downloaded.`);
  return true;
}

function lines(value) {
  return String(value)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function scheduleRefresh() {
  clearTimeout(state.refreshTimer);
  state.refreshTimer = setTimeout(() => {
    loadIncidents().catch(showError);
  }, 100);
}

function restartStream() {
  state.streamController?.abort();
  const controller = new AbortController();
  state.streamController = controller;
  const generation = ++state.streamGeneration;
  consumeStream(controller.signal, generation).catch((error) => {
    if (error.name !== 'AbortError') {
      showError(error);
    }
  });
}

async function consumeStream(signal, generation) {
  let delay = 500;
  while (!signal.aborted && generation === state.streamGeneration) {
    setStreamStatus('connecting', 'Connecting live stream…');
    try {
      const response = await fetch('/api/events', {
        headers: identityHeaders(),
        signal,
      });
      if (!response.ok) {
        const payload = await response.json();
        throw new ApiError(response, payload);
      }
      setStreamStatus('live', 'Live tenant stream connected');
      delay = 500;
      await readSse(response.body, signal, (event) => {
        if (event.event === 'connected') {
          setStreamStatus('live', 'Live tenant stream connected');
        } else {
          scheduleRefresh();
        }
      });
    } catch (error) {
      if (signal.aborted) {
        return;
      }
      setStreamStatus('reconnecting', 'Stream interrupted; reconnecting…');
      await sleep(delay, signal);
      delay = Math.min(delay * 2, 5_000);
    }
  }
}

async function readSse(stream, signal, onEvent) {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  try {
    while (!signal.aborted) {
      const { value, done } = await reader.read();
      if (done) {
        return;
      }
      buffer += decoder.decode(value, { stream: true });
      let boundary;
      while ((boundary = buffer.indexOf('\n\n')) !== -1) {
        const block = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        const event = parseSseBlock(block);
        if (event) {
          onEvent(event);
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function parseSseBlock(block) {
  if (!block || block.startsWith(':')) {
    return null;
  }
  const event = { event: 'message', data: null, id: null };
  const data = [];
  for (const line of block.split('\n')) {
    const separator = line.indexOf(':');
    const field = separator === -1 ? line : line.slice(0, separator);
    const value =
      separator === -1
        ? ''
        : line.slice(separator + 1).replace(/^ /, '');
    if (field === 'event') {
      event.event = value;
    } else if (field === 'id') {
      event.id = value;
    } else if (field === 'data') {
      data.push(value);
    }
  }
  if (data.length) {
    try {
      event.data = JSON.parse(data.join('\n'));
    } catch {
      event.data = data.join('\n');
    }
  }
  return event;
}

function sleep(milliseconds, signal) {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, milliseconds);
    signal.addEventListener(
      'abort',
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

function setStreamStatus(mode, text) {
  elements.streamDot.className = `status-dot ${mode}`;
  elements.streamStatus.textContent = text;
}

function showToast(message, warning = false) {
  elements.toast.textContent = message;
  elements.toast.className = warning ? 'toast warning' : 'toast';
  clearTimeout(showToast.timer);
  showToast.timer = setTimeout(() => {
    elements.toast.classList.add('hidden');
  }, 4_000);
}

function showError(error) {
  const correlation = error?.correlationId
    ? ` · correlation ${error.correlationId}`
    : '';
  showToast(`${error?.code ?? 'ERROR'}: ${error.message}${correlation}`, true);
  if (error instanceof ApiError) {
    console.warn(error);
  } else {
    console.error(error);
  }
}

function humanize(value) {
  return String(value)
    .replaceAll('_', ' ')
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

elements.identityForm.addEventListener('submit', (event) => {
  event.preventDefault();
  state.identity = {
    tenantId: elements.tenantId.value.trim(),
    actorId: elements.actorId.value.trim(),
    role: elements.role.value,
  };
  localStorage.setItem(
    'incident-command-identity',
    JSON.stringify(state.identity),
  );
  const generation = clearIncidentScope();
  applyPermissions();
  restartStream();
  loadIncidents({ preserveSelection: false, generation })
    .then((loaded) => {
      if (loaded) {
        showToast('Identity and tenant scope applied.');
      }
    })
    .catch(showError);
});
elements.refreshButton.addEventListener('click', () => {
  loadIncidents().catch(showError);
});
elements.filterSeverity.addEventListener('change', () => {
  loadIncidents().catch(showError);
});
elements.filterState.addEventListener('change', () => {
  loadIncidents().catch(showError);
});
elements.createForm.addEventListener('submit', (event) => {
  submitCreate(event).catch(showError);
});
elements.assignmentForm.addEventListener('submit', (event) => {
  submitAssignment(event).catch(showError);
});
elements.severityForm.addEventListener('submit', (event) => {
  submitSeverity(event).catch(showError);
});
elements.transitionForm.addEventListener('submit', (event) => {
  submitTransition(event).catch(showError);
});
elements.timelineForm.addEventListener('submit', (event) => {
  submitTimeline(event).catch(showError);
});
elements.postmortemForm.addEventListener('submit', (event) => {
  submitPostmortem(event).catch(showError);
});
elements.postmortemForm.addEventListener('input', () => {
  if (state.selected) {
    state.postmortemDraft = {
      incidentId: state.selected.id,
      dirty: true,
    };
  }
});
elements.previewButton.addEventListener('click', () => {
  previewPostmortem().catch(showError);
});
elements.downloadButton.addEventListener('click', () => {
  downloadPostmortem().catch(showError);
});

loadSavedIdentity();
applyPermissions();
restartStream();
loadIncidents({ preserveSelection: false }).catch(showError);
setInterval(() => {
  if (state.selected) {
    renderSloCard(elements.responseSlo, state.selected.slo.response);
    renderSloCard(elements.mitigationSlo, state.selected.slo.mitigation);
  }
}, 1_000);
