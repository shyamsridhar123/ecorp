/* Credit-policy exception workflow UI.
 * No frameworks, no network beyond this origin's own API.
 */
"use strict";

const state = {
  actor: null,
  actors: [],
  rules: [],
  exceptions: [],
  selectedId: null,
  detail: null,
  audit: [],
  verify: null,
};

let commandSeq = 0;
function nextCommandId(tag) {
  commandSeq += 1;
  return `ui-${tag}-${commandSeq}-${Math.random().toString(36).slice(2, 10)}`;
}

const $ = (sel) => document.querySelector(sel);

function banner(message, kind) {
  const el = $("#banner");
  el.textContent = message;
  el.className = `banner ${kind || ""}`.trim();
  el.hidden = false;
}

function clearBanner() {
  const el = $("#banner");
  el.hidden = true;
  el.textContent = "";
}

async function api(path, options = {}) {
  const headers = { Accept: "application/json" };
  if (state.actor) headers["X-Actor"] = state.actor;
  if (options.body !== undefined) headers["Content-Type"] = "application/json";
  if (options.commandId) headers["X-Command-Id"] = options.commandId;

  const res = await fetch(path, {
    method: options.method || "GET",
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });

  let data = null;
  const text = await res.text();
  if (text) {
    try { data = JSON.parse(text); } catch (_) { data = null; }
  }
  if (!res.ok) {
    const err = new Error((data && data.error && data.error.message) || `HTTP ${res.status}`);
    err.code = (data && data.error && data.error.code) || "http_error";
    err.status = res.status;
    err.details = data && data.error && data.error.details;
    throw err;
  }
  return data;
}

function esc(value) {
  return String(value == null ? "" : value).replace(/[&<>"']/g, (c) => (
    { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]
  ));
}

function labelize(value) {
  return String(value).replace(/_/g, " ");
}

/* --- bootstrap ------------------------------------------------------ */

async function loadActors() {
  const data = await api("/api/actors");
  state.actors = data.actors;
  const select = $("#actor-select");
  select.innerHTML = state.actors
    .map((a) => `<option value="${esc(a.actor_id)}">${esc(a.display_name)} — ${esc(a.tenant_id)} [${esc(a.roles.join(", "))}]</option>`)
    .join("");
  if (!state.actor) state.actor = state.actors[0].actor_id;
  select.value = state.actor;
  renderCaps();
}

function renderCaps() {
  const actor = state.actors.find((a) => a.actor_id === state.actor);
  $("#actor-caps").textContent = actor
    ? `${actor.actor_id} · ${actor.capabilities.join(", ")}`
    : "";
}

async function loadRules() {
  const data = await api("/api/policy-rules");
  state.rules = data.rules;
  $("#rule").innerHTML = state.rules
    .map((r) => `<option value="${esc(r.rule_id)}"${r.prohibited ? " data-prohibited=\"true\"" : ""}>${esc(r.rule_id)} — ${esc(r.title)}${r.prohibited ? " (no exceptions)" : ""}</option>`)
    .join("");
  updateRuleHint();
}

function updateRuleHint() {
  const rule = state.rules.find((r) => r.rule_id === $("#rule").value);
  $("#rule-hint").textContent = rule
    ? (rule.prohibited
      ? "This rule admits no exceptions; submission will be rejected."
      : `Ceiling ${rule.max_deviation_bps} bps · ${rule.requires_compliance ? "risk + compliance review" : "risk review only"}`)
    : "";
}

/* --- list ----------------------------------------------------------- */

async function loadExceptions() {
  const data = await api("/api/exceptions");
  state.exceptions = data.exceptions;
  renderList();
}

function renderList() {
  const list = $("#exception-list");
  if (!state.exceptions.length) {
    list.innerHTML = `<li class="empty" data-testid="list-empty">No exceptions for this tenant yet.</li>`;
    return;
  }
  list.innerHTML = state.exceptions.map((e) => `
    <li>
      <button type="button"
              class="exception-item${e.exception_id === state.selectedId ? " selected" : ""}"
              data-testid="exception-item"
              data-id="${esc(e.exception_id)}">
        <span class="row">
          <span class="id">${esc(e.exception_id)}</span>
          <span class="pill ${esc(e.state)}" data-testid="item-state">${esc(labelize(e.state))}</span>
        </span>
        <span class="meta">${esc(e.rule_id)} · ${esc(e.requested_deviation_bps)} bps · v${esc(e.version)} · ${esc(e.applicant_pseudonym)}</span>
      </button>
    </li>`).join("");
}

/* --- detail --------------------------------------------------------- */

async function selectException(id) {
  state.selectedId = id;
  try {
    state.detail = await api(`/api/exceptions/${encodeURIComponent(id)}`);
    const auditData = await api(`/api/exceptions/${encodeURIComponent(id)}/audit`);
    state.audit = auditData.entries;
  } catch (err) {
    state.detail = null;
    state.audit = [];
    banner(`${err.code}: ${err.message}`, "err");
  }
  renderList();
  renderDetail();
}

function renderDetail() {
  const host = $("#detail");
  const d = state.detail;
  if (!d) {
    host.innerHTML = `<p class="empty">Select an exception to view its workflow.</p>`;
    return;
  }
  const approvals = d.approvals.length
    ? d.approvals.map((a) => `
        <div class="approval${a.expired ? " expired" : ""}" data-testid="approval">
          <div class="who">${esc(labelize(a.kind))} · ${esc(a.decision)}${a.expired ? " · EXPIRED" : ""}</div>
          <div>${esc(a.actor_id)} — ${esc(a.rationale)}</div>
        </div>`).join("")
    : `<p class="empty">No approvals recorded.</p>`;

  const audit = state.audit.map((e) => `
    <div class="audit-entry" data-testid="audit-entry">
      <div>#${esc(e.sequence)} · ${esc(e.action)} · ${esc(e.actor_id)}</div>
      <code>${esc(e.entry_hash.slice(0, 24))}…</code>
    </div>`).join("");

  const v = state.verify;
  const verifyLine = v
    ? `<p data-testid="verify-result" class="${v.valid ? "hash-ok" : "hash-bad"}">Chain ${v.valid ? "VALID" : "BROKEN"} · ${esc(v.entry_count)} entries · tenant entries ${esc(v.tenant_entry_count)}</p>`
    : "";

  host.innerHTML = `
    <dl>
      <dt>Exception</dt><dd data-testid="detail-id">${esc(d.exception_id)}</dd>
      <dt>State</dt><dd><span class="pill ${esc(d.state)}" data-testid="detail-state">${esc(labelize(d.state))}</span></dd>
      <dt>Version</dt><dd data-testid="detail-version">${esc(d.version)}</dd>
      <dt>Rule</dt><dd>${esc(d.rule_id)}</dd>
      <dt>Deviation</dt><dd>${esc(d.requested_deviation_bps)} bps</dd>
      <dt>Applicant</dt><dd>${esc(d.applicant_pseudonym)}</dd>
      <dt>Requested by</dt><dd>${esc(d.requested_by)}</dd>
      <dt>Expires</dt><dd>${esc(d.expires_at)}</dd>
      <dt>Required</dt><dd>${esc(d.required_approval_kinds.map(labelize).join(", "))}</dd>
    </dl>
    <p class="section-title">Justification</p>
    <p>${esc(d.justification)}</p>
    <p class="section-title">Compensating controls</p>
    <ul>${d.compensating_controls.map((c) => `<li>${esc(c)}</li>`).join("")}</ul>
    <p class="section-title">Actions</p>
    <div class="actions">${actionButtons(d)}</div>
    <p class="section-title">Approvals</p>
    ${approvals}
    <p class="section-title">Audit chain</p>
    <div class="actions">
      <button type="button" class="ghost" data-testid="btn-verify" data-action="verify">Verify integrity</button>
    </div>
    ${verifyLine}
    ${audit}
  `;
}

function currentCapabilities() {
  const actor = state.actors.find((a) => a.actor_id === state.actor);
  return actor ? actor.capabilities : [];
}

function can(capability) {
  return currentCapabilities().includes(capability);
}

function actionButtons(d) {
  const buttons = [];
  const add = (action, label, testid) =>
    buttons.push(`<button type="button" data-action="${action}" data-testid="${testid}">${esc(label)}</button>`);

  // The server is authoritative; this only avoids offering actions the
  // current actor could never complete.
  const isOwner = d.requested_by === state.actor;

  if (d.state === "draft" && isOwner && can("exception:submit")) {
    add("submit", "Submit for review", "btn-submit");
  }
  if (d.state === "submitted" && can("exception:review_risk")) {
    add("analyze", "Run eligibility analysis", "btn-analyze");
  }
  if (d.state === "risk_review" && can("exception:review_risk")) {
    add("risk-approve", "Risk: approve", "btn-risk-approve");
    add("risk-reject", "Risk: reject", "btn-risk-reject");
  }
  if (d.state === "compliance_review" && can("exception:review_compliance")) {
    add("compliance-approve", "Compliance: approve", "btn-compliance-approve");
    add("compliance-reject", "Compliance: reject", "btn-compliance-reject");
  }
  if (d.state === "pending_decision" && can("exception:decide")) {
    add("decide-approve", "Authorize exception", "btn-decide-approve");
    add("decide-reject", "Decline exception", "btn-decide-reject");
  }
  if (isOwner && can("exception:withdraw")
      && ["draft", "submitted", "risk_review", "compliance_review", "pending_decision"].includes(d.state)) {
    add("withdraw", "Withdraw", "btn-withdraw");
  }
  if (!buttons.length) {
    return `<p class="empty" data-testid="no-actions">No actions available to you in state "${esc(labelize(d.state))}".</p>`;
  }
  return buttons.join("");
}

/* --- commands ------------------------------------------------------- */

const ACTION_MAP = {
  submit: { path: "submit", body: () => ({}) },
  analyze: { path: "analyze", body: () => ({}) },
  "risk-approve": { path: "risk-review", body: () => ({ decision: "approve", rationale: "Risk review approved via console." }) },
  "risk-reject": { path: "risk-review", body: () => ({ decision: "reject", rationale: "Risk review rejected via console." }) },
  "compliance-approve": { path: "compliance-review", body: () => ({ decision: "approve", rationale: "Compliance review approved via console." }) },
  "compliance-reject": { path: "compliance-review", body: () => ({ decision: "reject", rationale: "Compliance review rejected via console." }) },
  "decide-approve": { path: "decide", body: () => ({ decision: "approve", rationale: "Final authority approved via console." }) },
  "decide-reject": { path: "decide", body: () => ({ decision: "reject", rationale: "Final authority declined via console." }) },
  withdraw: { path: "withdraw", body: () => ({ reason: "Withdrawn by requester via console." }) },
};

async function runAction(action) {
  if (action === "verify") {
    try {
      state.verify = await api("/api/audit/verify");
      banner(`Audit chain ${state.verify.valid ? "verified" : "BROKEN"}.`,
             state.verify.valid ? "ok" : "err");
    } catch (err) {
      banner(`${err.code}: ${err.message}`, "err");
    }
    renderDetail();
    return;
  }

  const spec = ACTION_MAP[action];
  if (!spec || !state.detail) return;

  // The version we send is the one currently rendered. If the server has
  // moved on, it answers 409 and the UI surfaces the conflict.
  const body = Object.assign({ expected_version: state.detail.version }, spec.body());
  try {
    clearBanner();
    await api(`/api/exceptions/${encodeURIComponent(state.detail.exception_id)}/${spec.path}`, {
      method: "POST",
      body,
      commandId: nextCommandId(action),
    });
    banner(`Action "${labelize(action)}" completed.`, "ok");
  } catch (err) {
    banner(`${err.code}: ${err.message}`, "err");
  }
  await loadExceptions();
  await selectException(state.detail.exception_id);
}

async function createException(event) {
  event.preventDefault();
  clearBanner();
  const controls = $("#controls").value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const expiresRaw = $("#expires").value;
  const body = {
    applicant_pseudonym: $("#applicant").value.trim().toUpperCase(),
    rule_id: $("#rule").value,
    requested_deviation_bps: Number.parseInt($("#deviation").value, 10),
    justification: $("#justification").value.trim(),
    compensating_controls: controls,
    expires_at: expiresRaw ? `${expiresRaw}T00:00:00Z` : "",
  };
  try {
    const created = await api("/api/exceptions", {
      method: "POST",
      body,
      commandId: nextCommandId("create"),
    });
    banner(`Created ${created.exception_id}.`, "ok");
    $("#create-form").reset();
    updateRuleHint();
    await loadExceptions();
    await selectException(created.exception_id);
  } catch (err) {
    const detail = err.details ? ` (${JSON.stringify(err.details)})` : "";
    banner(`${err.code}: ${err.message}${detail}`, "err");
  }
}

/* --- wiring --------------------------------------------------------- */

function wire() {
  $("#create-form").addEventListener("submit", createException);
  $("#rule").addEventListener("change", updateRuleHint);
  $("#refresh").addEventListener("click", async () => {
    clearBanner();
    await loadExceptions();
    if (state.selectedId) await selectException(state.selectedId);
  });
  $("#actor-select").addEventListener("change", async (e) => {
    state.actor = e.target.value;
    state.verify = null;
    renderCaps();
    clearBanner();
    await loadExceptions();
    if (state.selectedId) await selectException(state.selectedId);
  });
  $("#exception-list").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-id]");
    if (btn) selectException(btn.dataset.id);
  });
  $("#detail").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-action]");
    if (btn) runAction(btn.dataset.action);
  });
}

async function boot() {
  try {
    await loadActors();
    await loadRules();
    await loadExceptions();
    wire();
    document.body.dataset.ready = "true";
  } catch (err) {
    banner(`Startup failed: ${err.message}`, "err");
  }
}

boot();
