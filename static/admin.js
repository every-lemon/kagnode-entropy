"use strict";
const $ = (selector) => document.querySelector(selector);
let csrf = "",
  active = false,
  epoch = 0;
function showProgress(label) {
  const progressEpoch = epoch;
  notify(label);
  const timer = setTimeout(() => {
    if (progressEpoch === epoch) window.KagNodeCore?.loading.show(label);
  }, 150);
  return () => {
    clearTimeout(timer);
    if (progressEpoch === epoch) window.KagNodeCore?.loading.hide();
  };
}
function notify(message) {
  const output = $(active ? "#message" : "#login-message");
  output.textContent = message;
  output.hidden = !message;
}
function clearSession() {
  epoch++;
  csrf = "";
  active = false;
  window.KagNodeLogin?.reset?.();
  window.KagNodeCore?.setSession({ authenticated: false });
  window.KagNodeCore?.loading.hide();
  $("#new-token").textContent = "";
  $("#clients").replaceChildren();
  $("#events").replaceChildren();
  document.querySelectorAll('input[type="password"]').forEach((input) => {
    input.value = "";
  });
  document.querySelectorAll("form button").forEach((button) => {
    button.disabled = false;
  });
  for (const [id, text] of [
    ["#owner-name", "Local administrator"],
    ["#owner-email", ""],
    ["#owner-profile", "Local administrator"],
  ]) {
    const node = $(id);
    if (node) node.textContent = text;
  }
  const photo = $("#owner-photo");
  if (photo) {
    photo.removeAttribute("src");
    photo.hidden = true;
  }
  $("#admin-panel").hidden = true;
  $("#admin-login").hidden = false;
}
async function api(path, body) {
  const requestEpoch = epoch;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 8000);
  try {
    const response = await fetch("/api/v1/admin/" + path, {
      method: body ? "POST" : "GET",
      credentials: "same-origin",
      cache: "no-store",
      signal: controller.signal,
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      ...(body ? { body: JSON.stringify(body) } : {}),
    });
    const value = await response.json();
    if (requestEpoch !== epoch)
      throw Object.assign(new Error("Superseded session request"), {
        stale: true,
      });
    if (
      !response.ok &&
      active &&
      (response.status === 401 ||
        value.error?.code === "configuration_unavailable")
    ) {
      clearSession();
      notify(value.error?.message || "Session expired. Sign in again.");
    }
    if (!response.ok)
      throw new Error(value.error?.message || "Request failed.");
    return value;
  } catch (error) {
    if (requestEpoch !== epoch) error.stale = true;
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}
function list(selector, items) {
  $(selector).replaceChildren(
    ...items.map((item) => {
      const li = document.createElement("li");
      li.textContent = item;
      return li;
    }),
  );
}
async function refresh() {
  const current = epoch;
  const status = await api("status");
  if (current !== epoch) return;
  csrf = status.csrf;
  active = true;
  $("#admin-login").hidden = true;
  $("#admin-panel").hidden = false;
  const engine = status.engine;
  $("#health").textContent = engine
    ? `Mode: ${engine.mode}\nReady blocks: ${engine.queued}\nGenerated: ${engine.produced}\nAllocated: ${engine.consumed}\nRNG failures: ${engine.source_failures}\nHealthy: ${!engine.producer_fault}`
    : "Engine unavailable";
  list("#clients", status.clients);
  list("#events", status.events);
  const account = await api("kagnode/settings");
  if (current !== epoch) return;
  $("#owner-profile").textContent = account.owner
    ? account.owner.name +
      (account.owner.email ? " · " + account.owner.email : "")
    : "Local administrator";
  $("#owner-name").textContent = account.owner?.name || "Local administrator";
  $("#owner-email").textContent = account.owner?.email || "";
  $("#sync-policy").value = account.enabled ? "on" : "off";
  window.KagNodeCore?.setSession({
    authenticated: true,
    subject: account.owner?.subject || "local-entropy-owner",
    name: account.owner?.name || "Local administrator",
    email: account.owner?.email || "",
  });
  if (account.owner?.photo) {
    $("#owner-photo").src = account.owner.photo;
    $("#owner-photo").hidden = false;
    $("#owner-initial").hidden = true;
  } else {
    $("#owner-photo").removeAttribute("src");
    $("#owner-photo").hidden = true;
    $("#owner-initial").hidden = false;
  }
  $("#appearance").value =
    document.documentElement.dataset.coreThemePreference || "system";
  $("#scroll-mode").value = window.KagNodeScroll?.mode || "custom";
  window.KagNodeControls?.sync();
}
$("#admin-login-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  epoch++;
  window.KagNodeLogin?.reset?.();
  const button = event.submitter;
  button.disabled = true;
  const operationEpoch = epoch;
  const finishProgress = showProgress("Signing in…");
  const credential = $("#credential").value;
  $("#credential").value = "";
  try {
    const value = await api("login", { credential });
    csrf = value.csrf;
    await refresh();
    notify("Signed in.");
  } catch (error) {
    if (!error.stale) notify(error.message);
  } finally {
    finishProgress();
    if (operationEpoch === epoch) button.disabled = false;
  }
});
$("#client-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = event.submitter;
  button.disabled = true;
  const operationEpoch = epoch;
  const finishProgress = showProgress("Updating client access…");
  const input = {
    id: $("#client-id").value,
    action: $("#action").value,
    credential: $("#confirm-key").value,
  };
  $("#confirm-key").value = "";
  $("#new-token").textContent = "";
  try {
    const value = await api("clients", input);
    $("#new-token").textContent = value.token || "Credential revoked.";
    notify("Client access updated.");
    await refresh();
  } catch (error) {
    if (!error.stale) notify(error.message);
  } finally {
    finishProgress();
    if (operationEpoch === epoch) button.disabled = false;
  }
});
$("#clear-token").addEventListener("click", () => {
  $("#new-token").textContent = "";
});
$("#signout").addEventListener("click", async () => {
  epoch++;
  window.KagNodeLogin?.reset?.();
  try {
    await api("logout", {});
  } catch (error) {
    if (!error.stale) notify(error.message);
    return;
  }
  clearSession();
  notify("Signed out.");
});
document.addEventListener("visibilitychange", () => {
  if (active && !document.hidden)
    refresh().catch((error) => {
      if (!error.stale)
        notify("Session expired or service unavailable. Sign in again.");
    });
});
refresh().catch(() => {});
window.KagNodeLoginHost = {
  async request(action, payload) {
    if (action === "start") epoch++;
    return api(
      "kagnode/" + (action === "status" ? "poll" : action),
      action === "start" ? {} : payload,
    );
  },
  async complete(value) {
    csrf = value.csrf;
    await refresh();
  },
};
fetch("/api/v1/admin/kagnode/availability", { cache: "no-store" })
  .then((r) => r.json())
  .then((value) => {
    const provider = document.querySelector("[data-core-login-provider]");
    provider.hidden = !value.enabled;
    provider.closest(".kn-core-login-provider").hidden = !value.enabled;
  })
  .catch(() => {});
$("#policy-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = event.submitter;
  button.disabled = true;
  const operationEpoch = epoch;
  const finishProgress = showProgress("Updating account settings…");
  const mode = $("#sync-policy").value,
    credential = $("#policy-key").value;
  $("#policy-key").value = "";
  try {
    await api("kagnode/policy", {
      credential,
      enabled: mode === "on",
      unlink: mode === "unlink",
    });
    location.assign("/admin");
  } catch (error) {
    if (!error.stale) notify(error.message);
  } finally {
    finishProgress();
    if (operationEpoch === epoch) button.disabled = false;
  }
});

$("#appearance").addEventListener("change", () =>
  window.KagNodeTheme?.saveLocal($("#appearance").value),
);

$("#scroll-mode").addEventListener("change", () =>
  window.KagNodeScroll?.setMode($("#scroll-mode").value),
);
document.addEventListener("kagnode:scroll-preference", () => {
  $("#scroll-mode").value = window.KagNodeScroll?.mode || "custom";
  window.KagNodeControls?.sync();
});
