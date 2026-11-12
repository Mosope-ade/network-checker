"use strict";
const $ = (id) => document.getElementById(id);
const escapeHTML = (value) =>
  String(value ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
let data = { targets: [], incidents: [] };
let serverOffset = 0;
let loaded = false;
let removeId = null;
let busy = false;
const labels = {
  up: "Responding",
  down: "Down",
  suspect: "Confirming failure",
  recovering: "Confirming recovery",
  pending: "Waiting",
  paused: "Paused",
  error: "Probe error",
};
function duration(ms) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  if (seconds < 86400)
    return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
  return `${Math.floor(seconds / 86400)}d ${Math.floor((seconds % 86400) / 3600)}h`;
}
const date = (ms) =>
  ms == null
    ? "—"
    : new Date(ms).toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      });
function showError(message) {
  $("error").textContent = message;
  $("error").hidden = !message;
}
async function request(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    signal: AbortSignal.timeout(8000),
  });
  if (!response.ok)
    throw new Error(
      (await response.text()) || `Request failed (${response.status})`,
    );
  return response.status === 204 ? null : response.json();
}
function renderTargets() {
  const query = $("search").value.toLowerCase().trim();
  const targets = data.targets.filter((t) =>
    `${t.name} ${t.address}`.toLowerCase().includes(query),
  );
  const active = document.activeElement;
  const focusId = active?.dataset.id;
  const focusAction = active?.dataset.action;
  $("targets").innerHTML = targets
    .map((t) => {
      const loss = t.sent
        ? (((t.sent - t.received) / t.sent) * 100).toFixed(1)
        : "—";
      const max = Math.max(0.001, ...t.history.filter((v) => v != null));
      const graph = t.history
        .map(
          (v) =>
            `<span class="probe${v == null ? " lost" : ` h${Math.max(1, Math.ceil((v / max) * 10))}`}" title="${v == null ? "No reply" : `${v.toFixed(2)} ms`}"></span>`,
        )
        .join("");
      return `<article class="target"><div class="target-header"><div><h2>${escapeHTML(t.name)}</h2><span class="address">${escapeHTML(t.address)}</span></div><span class="badge ${escapeHTML(t.status)}">${escapeHTML(labels[t.status] || t.status)}</span></div><div class="metrics"><div class="metric"><span>Last latency</span><strong>${t.latency_ms == null ? "—" : t.latency_ms.toFixed(2)} <small>ms</small></strong></div><div class="metric"><span>Packet loss</span><strong>${loss}<small>${t.sent ? " %" : ""}</small></strong></div><div class="metric"><span>Replies / sent</span><strong>${t.received}<small> / ${t.sent}</small></strong></div></div><div class="sparkline" role="img" aria-label="Last ${t.history.length} probes; ${t.history.filter((v) => v == null).length} without reply">${graph}</div><p class="target-error">${escapeHTML(t.error || (t.status === "recovering" ? `${t.tracker.successes} of ${t.recovery_threshold} successful probes to confirm recovery` : t.status === "paused" ? "Monitoring paused. Resume to check this target." : t.status === "pending" ? "Waiting for the first probe…" : ""))}</p><div class="target-footer"><span class="muted">Every ${t.interval}s · ${t.last_check ? new Date(t.last_check).toLocaleTimeString() : "Not checked"}</span><div class="actions"><button data-action="toggle" data-id="${t.id}" aria-label="${t.enabled ? "Pause" : "Resume"} ${escapeHTML(t.name)}">${t.enabled ? "Pause" : "Resume"}</button><button data-action="remove" data-id="${t.id}" aria-label="Remove ${escapeHTML(t.name)}">Remove</button></div></div></article>`;
    })
    .join("");
  if (focusId && focusAction)
    $("targets")
      .querySelector(`[data-id="${focusId}"][data-action="${focusAction}"]`)
      ?.focus({ preventScroll: true });
  $("empty").hidden = data.targets.length !== 0 || !loaded;
  if (data.targets.length && !targets.length)
    $("targets").innerHTML =
      '<p class="muted">No targets match your search.</p>';
}
function renderIncidents() {
  $("incidents").innerHTML = data.incidents
    .map(
      (i) =>
        `<tr><td>${escapeHTML(i.target_name)}<small>${escapeHTML(i.address)}</small></td><td>${date(i.started_at)}</td><td>${date(i.ended_at)}</td><td>${duration((i.ended_at ?? Date.now() + serverOffset) - i.started_at)}</td><td><span class="badge ${i.outcome === "ongoing" ? "down" : i.outcome === "recovered" ? "up" : "paused"}">${escapeHTML(i.outcome)}</span></td><td>${escapeHTML(i.reason)}</td></tr>`,
    )
    .join("");
  $("no-incidents").hidden = data.incidents.length !== 0;
  document.querySelector(".table-wrap").hidden = data.incidents.length === 0;
}
function render() {
  $("total").textContent = data.targets.length;
  $("up").textContent = data.targets.filter(
    (t) => t.enabled && t.status === "up",
  ).length;
  $("down").textContent = data.targets.filter(
    (t) => t.enabled && t.tracker.down,
  ).length;
  $("open").textContent = data.open_incidents;
  $("target-count").textContent = data.targets.length;
  $("incident-count").textContent = data.incidents.length;
  renderTargets();
  renderIncidents();
}
async function refresh() {
  try {
    data = await request("/api/status");
    serverOffset = data.server_time - Date.now();
    loaded = true;
    $("connection").textContent = "Connected · local session";
    $("connection").className = "connected";
    $("updated").textContent = `Updated ${new Date().toLocaleTimeString()}`;
    showError("");
    render();
  } catch (error) {
    $("connection").textContent = "Disconnected · retrying";
    $("connection").className = "";
    showError(
      `Cannot reach NetCheck. Displayed results may be stale. Keep the server running. ${error.message}`,
    );
  }
}
async function poll() {
  await refresh();
  setTimeout(poll, 1000);
}
function openAdd() {
  $("form-error").hidden = true;
  $("add-dialog").showModal();
  $("addresses").focus();
}
$("add-open").addEventListener("click", openAdd);
$("empty-add").addEventListener("click", openAdd);
for (const id of ["add-close", "add-cancel"])
  $(id).addEventListener("click", () => {
    if (!busy) $("add-dialog").close();
  });
$("add-dialog").addEventListener("cancel", (event) => {
  if (busy) event.preventDefault();
});
$("search").addEventListener("input", renderTargets);
$("add-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const lines = $("addresses")
    .value.split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  if (!lines.length) return;
  busy = true;
  $("add-submit").disabled = true;
  $("form-error").hidden = true;
  let completed = 0;
  try {
    for (const line of lines) {
      const comma = line.indexOf(",");
      const address = (comma < 0 ? line : line.slice(0, comma)).trim();
      const name =
        (comma < 0 ? address : line.slice(comma + 1)).trim() || address;
      $("add-submit").textContent =
        `Adding ${completed + 1} / ${lines.length}…`;
      await request("/api/targets", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name,
          address,
          interval: Number($("interval").value),
          failure_threshold: Number($("failures").value),
          recovery_threshold: Number($("recoveries").value),
        }),
      });
      completed++;
    }
    $("addresses").value = "";
    $("add-dialog").close();
  } catch (error) {
    $("addresses").value = lines.slice(completed).join("\n");
    $("form-error").textContent =
      `${completed ? `${completed} target(s) added. ` : ""}${error.message}`;
    $("form-error").hidden = false;
  } finally {
    busy = false;
    $("add-submit").disabled = false;
    $("add-submit").textContent = "Start monitoring";
    await refresh();
  }
});
$("targets").addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;
  const target = data.targets.find((t) => t.id === Number(button.dataset.id));
  if (!target) return;
  if (button.dataset.action === "remove") {
    removeId = target.id;
    $("remove-text").textContent = `${target.name} · ${target.address}`;
    $("remove-dialog").returnValue = "";
    $("remove-dialog").showModal();
    $("remove-dialog").querySelector('[value="cancel"]').focus();
  } else {
    button.disabled = true;
    try {
      await request(`/api/targets/${target.id}/toggle`, { method: "POST" });
      await refresh();
    } catch (error) {
      showError(error.message);
      button.disabled = false;
    }
  }
});
$("remove-dialog").addEventListener("close", async () => {
  if ($("remove-dialog").returnValue !== "confirm" || removeId == null) return;
  try {
    await request(`/api/targets/${removeId}`, { method: "DELETE" });
    await refresh();
    $("add-open").focus();
  } catch (error) {
    showError(error.message);
  }
  removeId = null;
});
function selectTab(view) {
  for (const name of ["targets", "incidents"]) {
    const selected = name === view;
    $(`${name}-tab`).setAttribute("aria-selected", String(selected));
    $(`${name}-tab`).tabIndex = selected ? 0 : -1;
    $(`${name}-panel`).hidden = !selected;
  }
}
for (const name of ["targets", "incidents"]) {
  $(`${name}-tab`).addEventListener("click", () => selectTab(name));
  $(`${name}-tab`).addEventListener("keydown", (event) => {
    if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const next =
        event.key === "Home"
          ? "targets"
          : event.key === "End"
            ? "incidents"
            : name === "targets"
              ? "incidents"
              : "targets";
      selectTab(next);
      $(`${next}-tab`).focus();
    }
  });
}
poll();
