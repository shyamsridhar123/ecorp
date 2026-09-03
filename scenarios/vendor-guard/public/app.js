const state = {
  context: {
    tenantId: localStorage.getItem("vg.tenant") ?? "acme",
    actorId: localStorage.getItem("vg.actor") ?? "requester.alex",
    role: localStorage.getItem("vg.role") ?? "requester",
  },
  vendors: [],
};

const elements = {
  tenant: document.querySelector("#tenant"),
  actor: document.querySelector("#actor"),
  role: document.querySelector("#role"),
  apply: document.querySelector("#apply-context"),
  notice: document.querySelector("#notice"),
  form: document.querySelector("#vendor-form"),
  intake: document.querySelector("#intake-panel"),
  list: document.querySelector("#vendor-list"),
  template: document.querySelector("#vendor-template"),
  refresh: document.querySelector("#refresh"),
  pending: document.querySelector("#pending-count"),
  auditPanel: document.querySelector("#audit-panel"),
  auditButton: document.querySelector("#verify-audit"),
  auditResult: document.querySelector("#audit-result"),
};

elements.tenant.value = state.context.tenantId;
elements.actor.value = state.context.actorId;
elements.role.value = state.context.role;

function headers(extra = {}) {
  return {
    "Content-Type": "application/json",
    "X-Tenant-Id": state.context.tenantId,
    "X-Actor-Id": state.context.actorId,
    "X-Role": state.context.role,
    ...extra,
  };
}

async function api(path, options = {}) {
  const response = await fetch(path, { ...options, headers: headers(options.headers) });
  const payload = await response.json();
  if (!response.ok) {
    const error = new Error(payload.error?.message ?? "Request failed");
    error.code = payload.error?.code ?? "UNKNOWN_ERROR";
    error.details = payload.error?.details;
    throw error;
  }
  return payload;
}

function showNotice(message, isError = false) {
  elements.notice.textContent = message;
  elements.notice.classList.toggle("error", isError);
  elements.notice.hidden = false;
}

function clearNotice() {
  elements.notice.hidden = true;
  elements.notice.textContent = "";
}

function describeError(error) {
  if (error.code === "VERSION_CONFLICT") {
    const current = error.details?.currentVersion;
    return `Conflict detected: this view is stale${current ? ` (current version ${current})` : ""}. Refresh before retrying.`;
  }
  if (error.code === "MAKER_CHECKER_REQUIRED") {
    return "Maker-checker denied: choose an independent reviewer for this high-risk submission.";
  }
  if (error.code === "IDEMPOTENCY_CONFLICT") {
    return "Idempotency conflict: that key already protects a different submission.";
  }
  return `${error.code}: ${error.message}`;
}

function button(label, className, action) {
  const control = document.createElement("button");
  control.type = "button";
  control.className = `button ${className}`;
  control.textContent = label;
  control.addEventListener("click", action);
  return control;
}

async function submitVendor(vendor) {
  clearNotice();
  try {
    const result = await api(`/api/vendors/${vendor.id}/submit`, {
      method: "POST",
      headers: { "Idempotency-Key": `ui-${vendor.id}-${vendor.version}` },
      body: JSON.stringify({ expectedVersion: vendor.version }),
    });
    showNotice(
      result.idempotentReplay
        ? "Submission replay returned the original durable result."
        : "Assessment submitted for risk review.",
    );
    await loadVendors();
  } catch (error) {
    showNotice(describeError(error), true);
  }
}

async function reviewVendor(vendor, decision, reasonInput) {
  clearNotice();
  try {
    await api(`/api/vendors/${vendor.id}/reviews`, {
      method: "POST",
      body: JSON.stringify({
        expectedVersion: vendor.version,
        decision,
        reason: reasonInput.value,
      }),
    });
    showNotice(`Review decision recorded: ${decision.replace("_", " ")}.`);
    await loadVendors();
  } catch (error) {
    showNotice(describeError(error), true);
  }
}

function renderVendor(vendor) {
  const card = elements.template.content.firstElementChild.cloneNode(true);
  card.dataset.vendorId = vendor.id;
  const status = card.querySelector(".status-pill");
  status.textContent = vendor.status.replaceAll("_", " ");
  status.classList.add(vendor.status);
  card.querySelector(".version").textContent = `v${vendor.version}`;
  card.querySelector(".vendor-title").textContent = vendor.name;
  card.querySelector(".vendor-service").textContent = vendor.service;
  card.querySelector(".risk-score").textContent = vendor.risk.score;
  card.querySelector(".risk-level").textContent = `${vendor.risk.level} risk`;
  card.querySelector(".thresholds").textContent =
    `Thresholds: low ${vendor.risk.thresholds.low} · medium ${vendor.risk.thresholds.medium} · high ${vendor.risk.thresholds.high}` +
    (vendor.risk.thresholds.makerCheckerRequired ? " · independent reviewer required" : "");

  const factors = card.querySelector(".factor-list");
  for (const factor of vendor.risk.factors) {
    const item = document.createElement("li");
    const explanation = document.createElement("span");
    const points = document.createElement("b");
    explanation.textContent = factor.explanation;
    points.textContent = `+${factor.points}`;
    item.append(explanation, points);
    factors.append(item);
  }

  const history = card.querySelector(".review-history");
  for (const review of vendor.reviewHistory) {
    const note = document.createElement("p");
    note.className = "review-note";
    const heading = document.createElement("strong");
    heading.textContent = `${review.decision.replace("_", " ")} by ${review.actorId}: `;
    note.append(heading, document.createTextNode(review.reason));
    history.append(note);
  }

  const actions = card.querySelector(".card-actions");
  if (
    state.context.role === "requester" &&
    ["draft", "changes_requested"].includes(vendor.status)
  ) {
    actions.append(button("Submit for review", "primary submit-vendor", () => submitVendor(vendor)));
  } else if (state.context.role === "reviewer" && vendor.status === "pending_review") {
    const reason = document.createElement("input");
    reason.className = "review-reason";
    reason.placeholder = "Durable review reasoning (minimum 10 characters)";
    reason.maxLength = 1000;
    reason.setAttribute("aria-label", `Review reason for ${vendor.name}`);
    actions.append(
      reason,
      button("Approve", "primary approve-vendor", () => reviewVendor(vendor, "approve", reason)),
      button("Request changes", "secondary request-changes", () =>
        reviewVendor(vendor, "request_changes", reason),
      ),
      button("Reject", "danger reject-vendor", () => reviewVendor(vendor, "reject", reason)),
    );
  } else {
    const message = document.createElement("span");
    message.className = "boundary-note";
    message.textContent = TERMINAL_TEXT[vendor.status] ?? "No actions available for this role.";
    actions.append(message);
  }
  return card;
}

const TERMINAL_TEXT = {
  approved: "Approved records are terminal and immutable.",
  rejected: "Rejected records are terminal and immutable.",
};

function render() {
  elements.intake.hidden = state.context.role !== "requester";
  elements.auditPanel.hidden = state.context.role !== "admin";
  elements.pending.textContent = state.vendors.filter(
    (vendor) => vendor.status === "pending_review",
  ).length;
  elements.list.replaceChildren();
  if (state.vendors.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = `No vendor assessments are visible in ${state.context.tenantId}.`;
    elements.list.append(empty);
    return;
  }
  for (const vendor of state.vendors) elements.list.append(renderVendor(vendor));
}

async function loadVendors() {
  clearNotice();
  try {
    const payload = await api("/api/vendors");
    state.vendors = payload.vendors;
    render();
  } catch (error) {
    showNotice(describeError(error), true);
  }
}

elements.apply.addEventListener("click", async () => {
  state.context = {
    tenantId: elements.tenant.value,
    actorId: elements.actor.value.trim(),
    role: elements.role.value,
  };
  localStorage.setItem("vg.tenant", state.context.tenantId);
  localStorage.setItem("vg.actor", state.context.actorId);
  localStorage.setItem("vg.role", state.context.role);
  elements.auditResult.textContent = "Not yet verified";
  elements.auditResult.className = "audit-result";
  await loadVendors();
});

elements.refresh.addEventListener("click", loadVendors);

elements.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  clearNotice();
  const payload = {
    name: document.querySelector("#vendor-name").value,
    service: document.querySelector("#vendor-service").value,
    assessment: {
      dataClassification: document.querySelector("#data-classification").value,
      criticality: document.querySelector("#criticality").value,
      annualSpend: Number(document.querySelector("#annual-spend").value),
      internetExposure: document.querySelector("#internet-exposure").checked,
      securityReview: document.querySelector("#security-review").checked,
    },
  };
  try {
    await api("/api/vendors", { method: "POST", body: JSON.stringify(payload) });
    elements.form.reset();
    document.querySelector("#annual-spend").value = "50000";
    showNotice("Draft assessment created with a deterministic risk score.");
    await loadVendors();
  } catch (error) {
    showNotice(describeError(error), true);
  }
});

elements.auditButton.addEventListener("click", async () => {
  clearNotice();
  try {
    const { verification } = await api("/api/audit/verify");
    elements.auditResult.className = `audit-result ${verification.valid ? "valid" : "invalid"}`;
    elements.auditResult.textContent = verification.valid
      ? `VALID · ${verification.eventCount} events · head ${verification.headHash ?? "empty chain"}`
      : `INVALID · chain break at ${verification.brokenAt ?? "unknown event"}`;
  } catch (error) {
    showNotice(describeError(error), true);
  }
});

await loadVendors();
