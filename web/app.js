"use strict";
const $ = (id) => document.getElementById(id);
// User-supplied content is always assigned as text, never interpreted as HTML.
function element(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attributes)) {
    if (key === "className") node.className = value;
    else if (key === "disabled") node.disabled = value;
    else node.setAttribute(key, value);
  }
  node.append(...children.filter((child) => child != null));
  return node;
}
const pendingActions = new Set();
let refreshSequence = 0;
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
  stale: "Monitoring delayed",
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
function isStale(target) {
  return (
    target.enabled &&
    target.last_check != null &&
    Date.now() + serverOffset - target.last_check > (target.interval + 7) * 1000
  );
}
function renderTargets() {
  const query = $("search").value.toLowerCase().trim();
  const targets = data.targets.filter((t) =>
    `${t.name} ${t.address}`.toLowerCase().includes(query),
  );
  const active = document.activeElement;
  const focusId = active?.dataset.id;
  const focusAction = active?.dataset.action;
  const cards = targets.map((t) => {
    const status = isStale(t) ? "stale" : t.status;
    const loss = t.sent
      ? (((t.sent - t.received) / t.sent) * 100).toFixed(1)
      : "—";
    const metric = (label, value, suffix) =>
      element(
        "div",
        { className: "metric" },
        element("span", {}, label),
        element("strong", {}, String(value), element("small", {}, suffix)),
      );
    const max = Math.max(0.001, ...t.history.filter((v) => v != null));
    const bars = t.history.map((v) =>
      element("span", {
        className:
          v == null
            ? "probe lost"
            : `probe h${Math.max(1, Math.ceil((v / max) * 10))}`,
        title: v == null ? "No reply" : `${v.toFixed(2)} ms`,
      }),
    );
    const button = (action, label) =>
      element(
        "button",
        {
          "data-id": String(t.id),
          "data-action": action,
          "aria-label": `${label} ${t.name}`,
          disabled: pendingActions.has(t.id),
        },
        label,
      );
    const detail = isStale(t)
      ? "No recent probe result. Check the server log."
      : t.error ||
        (t.status === "recovering"
          ? `${t.tracker.successes} of ${t.recovery_threshold} successful probes to confirm recovery`
          : t.status === "paused"
            ? "Monitoring paused. Resume to check this target."
            : t.status === "pending"
              ? "Waiting for the first probe…"
              : "");
    return element(
      "article",
      { className: "target" },
      element(
        "div",
        { className: "target-header" },
        element(
          "div",
          {},
          element("h2", {}, t.name),
          element("span", { className: "address" }, t.address),
        ),
        element(
          "span",
          { className: `badge ${status}` },
          labels[status] || status,
        ),
      ),
      element(
        "div",
        { className: "metrics" },
        metric(
          "Last latency",
          t.latency_ms == null ? "—" : t.latency_ms.toFixed(2),
          " ms",
        ),
        metric("Packet loss", loss, t.sent ? " %" : ""),
        metric("Replies / sent", t.received, ` / ${t.sent}`),
      ),
      element(
        "div",
        {
          className: "sparkline",
          role: "img",
          "aria-label": `Last ${t.history.length} probes; ${t.history.filter((v) => v == null).length} without reply`,
        },
        ...bars,
      ),
      element("p", { className: "target-error" }, detail),
      element(
        "div",
        { className: "target-footer" },
        element(
          "span",
          { className: "muted" },
          `Every ${t.interval}s · ${t.last_check == null ? "Not checked" : new Date(t.last_check).toLocaleTimeString()}`,
        ),
        element(
          "div",
          { className: "actions" },
          button("state", t.enabled ? "Pause" : "Resume"),
          button("remove", "Remove"),
        ),
      ),
    );
  });
  $("targets").replaceChildren(...cards);
  if (focusId && focusAction)
    $("targets")
      .querySelector(`[data-id="${focusId}"][data-action="${focusAction}"]`)
      ?.focus({ preventScroll: true });
  $("empty").hidden = data.targets.length !== 0 || !loaded;
  if (data.targets.length && !targets.length)
    $("targets").replaceChildren(
      element("p", { className: "muted" }, "No targets match your search."),
    );
}
function renderIncidents() {
  $("incidents").replaceChildren(
    ...data.incidents.map((i) => {
      const badge = element(
        "span",
        {
          className: `badge ${i.outcome === "ongoing" ? "down" : i.outcome === "recovered" ? "up" : "paused"}`,
        },
        i.outcome,
      );
      return element(
        "tr",
        {},
        element("td", {}, i.target_name, element("small", {}, i.address)),
        element("td", {}, date(i.started_at)),
        element("td", {}, date(i.ended_at)),
        element(
          "td",
          {},
          duration((i.ended_at ?? Date.now() + serverOffset) - i.started_at),
        ),
        element("td", {}, badge),
        element("td", {}, i.reason),
      );
    }),
  );
  $("no-incidents").hidden = data.incidents.length !== 0;
  document.querySelector(".table-wrap").hidden = data.incidents.length === 0;
}
function render() {
  $("total").textContent = data.targets.length;
  $("up").textContent = data.targets.filter(
    (t) => t.enabled && t.status === "up" && !isStale(t),
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
  const sequence = ++refreshSequence;
  try {
    const result = await request("/api/status");
    if (sequence !== refreshSequence) return;
    data = result;
    serverOffset = data.server_time - Date.now();
    loaded = true;
    $("connection").textContent = "Connected · local session";
    $("connection").className = "connected";
    $("updated").textContent = `Updated ${new Date().toLocaleTimeString()}`;
    $("connection-error").hidden = true;
    render();
  } catch (error) {
    if (sequence !== refreshSequence) return;
    $("connection").textContent = "Disconnected · retrying";
    $("connection").className = "";
    $("connection-error").textContent =
      `Cannot reach NetCheck. Displayed results may be stale. Keep the server running. ${error.message}`;
    $("connection-error").hidden = false;
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
  if (!lines.length) {
    $("form-error").textContent = "Enter at least one target address.";
    $("form-error").hidden = false;
    return;
  }
  if (busy) return;
  const settings = {
    interval: Number($("interval").value),
    failure_threshold: Number($("failures").value),
    recovery_threshold: Number($("recoveries").value),
  };
  const fields = [...$("add-form").querySelectorAll("input, textarea, button")];
  fields.forEach((field) => (field.disabled = true));
  showError("");
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
          ...settings,
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
    fields.forEach((field) => (field.disabled = false));
    $("add-submit").disabled = false;
    $("add-submit").textContent = "Start monitoring";
    await refresh();
  }
});
$("targets").addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;
  const target = data.targets.find((t) => t.id === Number(button.dataset.id));
  if (!target || pendingActions.has(target.id)) return;
  showError("");
  if (button.dataset.action === "remove") {
    removeId = target.id;
    $("remove-text").textContent = `${target.name} · ${target.address}`;
    $("remove-dialog").returnValue = "";
    $("remove-dialog").showModal();
    $("remove-dialog").querySelector('[value="cancel"]').focus();
  } else {
    pendingActions.add(target.id);
    renderTargets();
    try {
      await request(`/api/targets/${target.id}/state`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ enabled: !target.enabled }),
      });
      await refresh();
    } catch (error) {
      showError(error.message);
    } finally {
      pendingActions.delete(target.id);
      renderTargets();
    }
  }
});
$("remove-dialog").addEventListener("close", async () => {
  if ($("remove-dialog").returnValue !== "confirm" || removeId == null) return;
  const id = removeId;
  removeId = null;
  pendingActions.add(id);
  renderTargets();
  showError("");
  try {
    await request(`/api/targets/${id}`, { method: "DELETE" });
    await refresh();
    $("add-open").focus();
  } catch (error) {
    showError(error.message);
  }
  pendingActions.delete(id);
  renderTargets();
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
