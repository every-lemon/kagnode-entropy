/*! Local-administrator Core status and reviewed UI actions. */
(() => {
  const core = window.KagNodeCore;
  if (!core) return;
  let generation = 0,
    readGeneration = 0,
    readController,
    navigating = false;
  const requests = new Set();
  const blocked = () =>
    navigating ||
    document.hidden ||
    window.KagNodeLoading?.pending > 0 ||
    document.documentElement.matches(".kag-loading-covered,.page-loading");
  function interrupted() {
    return new DOMException(
      "Core request interrupted; inspect status before trying again.",
      "AbortError",
    );
  }
  function assertActive(context) {
    if (
      context.epoch !== generation ||
      context.controller.signal.aborted ||
      navigating
    )
      throw interrupted();
  }
  function abortRead() {
    readGeneration++;
    readController?.abort();
    readController = null;
    checking = false;
    checked = 0;
  }
  function navigationIntent(event) {
    if (event.type === "click") {
      if (
        event.button ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.altKey
      )
        return;
      const link = event.target.closest?.("a[href]");
      if (
        !link ||
        link.hasAttribute("download") ||
        (link.target && link.target !== "_self")
      )
        return;
      let url;
      try {
        url = new URL(link.href, location.href);
      } catch {
        return;
      }
      if (
        !/^https?:$/.test(url.protocol) ||
        (url.origin === location.origin &&
          url.pathname === location.pathname &&
          url.search === location.search &&
          url.hash)
      )
        return;
    } else {
      const form = event.target;
      if (
        !(form instanceof HTMLFormElement) ||
        form.method === "dialog" ||
        ((event.submitter?.formTarget || form.target) &&
          (event.submitter?.formTarget || form.target) !== "_self")
      )
        return;
    }
    stopNavigation();
    setTimeout(() => {
      if (event.defaultPrevented) {
        navigating = false;
        refresh();
      }
    });
  }
  function stopNavigation() {
    navigating = true;
    abortRead();
    for (const controller of requests) controller.abort();
  }
  let current,
    checking = false,
    checked = 0,
    panel;
  async function headers(signal) {
    const result = { Accept: "application/json" },
      app = document.body.dataset.knAccountApp;
    if (app === "httpboot") {
      const t = sessionStorage.getItem("kagnode.httpboot.console");
      if (t) result.Authorization = "Bearer " + t;
    } else if (app === "vpn")
      result["X-CSRF-Token"] =
        document.querySelector("input[name=csrf]")?.value || "";
    else if (app === "media")
      result["X-CSRF-Token"] =
        document.querySelector('meta[name="csrf-token"]')?.content || "";
    else if (app === "authenticator" || app === "mail") {
      if (app === "authenticator") result["X-Requested-With"] = "Authentactor";
      const r = await fetch(app === "mail" ? "/api/session" : "/api/status", {
        credentials: "same-origin",
        cache: "no-store",
        signal,
      });
      if (!r.ok) throw Error("Sign in locally to manage Core.");
      result["X-CSRF-Token"] = (await r.json()).csrf || "";
    } else
      result["X-CSRF-Token"] =
        document.querySelector("input[name=_csrf]")?.value || "";
    return result;
  }
  async function request(
    action = "status",
    data = {},
    allowReauth = true,
    context = { controller: new AbortController(), epoch: generation },
  ) {
    requests.add(context.controller);
    try {
      assertActive(context);
      const preparedHeaders = await headers(context.controller.signal);
      assertActive(context);
      const r = await fetch("/api/v1/kagnode-core/" + action, {
        method: action === "status" ? "GET" : "POST",
        credentials: "same-origin",
        cache: "no-store",
        signal: context.controller.signal,
        headers: {
          ...preparedHeaders,
          ...(action === "status"
            ? {}
            : { "Content-Type": "application/json" }),
        },
        ...(action === "status" ? {} : { body: JSON.stringify(data) }),
      });
      const v = await r
        .json()
        .catch(() => ({ error: "Core status is unavailable." }));
      assertActive(context);
      if (
        r.status === 403 &&
        v.code === "reauth_required" &&
        allowReauth &&
        window.KagNodeCoreHost?.reauthenticate
      ) {
        context.onReauth?.();
        await window.KagNodeCoreHost.reauthenticate();
        assertActive(context);
        return request(action, data, false, context);
      }
      if (!r.ok) {
        const e = Error(v.error || "Core request failed.");
        e.status = r.status;
        throw e;
      }
      return v;
    } finally {
      requests.delete(context.controller);
    }
  }
  const el = (tag, text) => {
    const n = document.createElement(tag);
    if (text) n.textContent = text;
    return n;
  };
  function render() {
    if (!current || !window.KagNodeTheme.session.authenticated) return;
    const host =
      document.querySelector("[data-core-admin-host],.kn-account-settings") ||
      document.querySelector("[data-core-preferences-panel]")?.parentElement;
    if (!host || host.closest("[hidden]")) return;
    if (!panel?.isConnected) {
      panel = el("section");
      panel.className = "kn-core-preferences";
      panel.dataset.coreUpdatesPanel = "";
      host.append(panel);
    }
    panel.replaceChildren(
      el("h3", "KagNode Core"),
      el("p", "UI components · " + (current.version || core.version)),
    );
    const status = el(
      "p",
      current.message ||
        current.last_error ||
        current.operation?.message ||
        "Components are served by this server.",
    );
    status.setAttribute("role", "status");
    panel.append(status);
    if (!current.enabled) return;
    const candidate = current.candidate;
    if (candidate?.version)
      panel.append(
        el(
          "p",
          candidate.changed
            ? "Available: " + candidate.version
            : "You have the current compatible Core version.",
        ),
      );
    const actions = el("div");
    actions.className = "kn-core-preference-options";
    panel.append(actions);
    function button(label, action, data, confirm) {
      const b = el("button", label);
      b.type = "button";
      b.onclick = async () => {
        const epoch = generation;
        if (confirm) {
          const proceed = await review(confirm);
          if (!proceed || epoch !== generation || navigating) return;
        }
        b.disabled = true;
        const finish = core.loading.begin({
          mode: "overlay",
          label:
            action === "check"
              ? "Checking UI updates…"
              : action === "apply"
                ? "Updating UI components…"
                : action === "rollback"
                  ? "Restoring UI components…"
                  : "Updating preference…",
        });
        try {
          const value = await request(action, data, true, {
            controller: new AbortController(),
            epoch,
            onReauth: finish,
          });
          if (epoch !== generation) return;
          current = value;
          render();
        } catch (e) {
          if (epoch === generation && !navigating)
            status.textContent =
              e.name === "AbortError"
                ? "Request interrupted. Inspect status before trying again."
                : e.message;
        } finally {
          b.disabled = false;
          finish();
        }
      };
      actions.append(b);
    }
    button("Check for updates", "check");
    if (candidate?.changed)
      button(
        "Install " + candidate.version,
        "apply",
        { release_id: candidate.release_id },
        "Install the verified Core UI patch " +
          candidate.version +
          "? Your application service will keep running.",
      );
    if (current.capabilities?.update_summaries)
      button(
        current.summaries !== false
          ? "Disable user update summaries"
          : "Enable user update summaries",
        "policy",
        { summaries: current.summaries === false },
      );
    button(
      current.automatic
        ? "Pause automatic patches"
        : "Enable automatic patches",
      "policy",
      { automatic: !current.automatic },
    );
    if (current.previous)
      button(
        "Restore previous UI",
        "rollback",
        {},
        "Restore the previous UI version and pause automatic patches?",
      );
    const link = el("a", "Release notes");
    link.href = "https://github.com/every-lemon/kagnode-core/releases";
    link.target = "_blank";
    link.rel = "noopener noreferrer";
    panel.append(link);
  }
  function review(message) {
    return new Promise((resolve) => {
      const d = el("dialog"),
        p = el("p", message),
        actions = el("div"),
        cancel = el("button", "Cancel"),
        ok = el("button", "Continue");
      d.className = "kn-core-preferences";
      cancel.type = ok.type = "button";
      actions.className = "kn-core-preference-options";
      actions.append(cancel, ok);
      d.append(p, actions);
      document.body.append(d);
      let result = false;
      cancel.onclick = () => d.close();
      ok.onclick = () => {
        result = true;
        d.close();
      };
      d.addEventListener(
        "close",
        () => {
          d.remove();
          resolve(result);
        },
        { once: true },
      );
      d.showModal();
      cancel.focus();
    });
  }
  async function refresh() {
    if (
      checking ||
      blocked() ||
      Date.now() - checked < 5000 ||
      !window.KagNodeTheme.session.authenticated
    )
      return;
    checking = true;
    const epoch = generation,
      attempt = readGeneration;
    readController = new AbortController();
    try {
      const value = await request("status", {}, false, {
        controller: readController,
        epoch,
      });
      if (epoch !== generation || attempt !== readGeneration || blocked())
        return;
      current = value;
      render();
    } catch (e) {
      if (
        epoch === generation &&
        attempt === readGeneration &&
        [401, 403, 404].includes(e.status)
      ) {
        current = null;
        panel?.remove();
        panel = null;
      }
    } finally {
      if (epoch === generation && attempt === readGeneration) {
        checking = false;
        checked = Date.now();
      }
    }
  }
  document.addEventListener("kagnode:core-session", () => {
    generation++;
    abortRead();
    for (const controller of requests) controller.abort();
    current = null;
    panel?.remove();
    panel = null;
    checked = 0;
    refresh();
  });
  document.addEventListener("kagnode:account-ready", refresh);
  document.addEventListener("click", navigationIntent, true);
  document.addEventListener("submit", navigationIntent, true);
  addEventListener("beforeunload", stopNavigation);
  addEventListener("pagehide", stopNavigation);
  addEventListener("pageshow", () => {
    navigating = false;
    refresh();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) abortRead();
    else refresh();
  });
  document.addEventListener("kagnode:loading-start", abortRead);
  document.addEventListener("kagnode:loading-end", refresh);
  document.addEventListener("kagnode:loading-operation", (event) => {
    if (event.detail?.pending) abortRead();
    else refresh();
  });
  addEventListener("focus", () => {
    checked = 0;
    refresh();
  });
  setInterval(() => {
    render();
    if (!document.hidden) refresh();
  }, 30000);
  window.KagNodeCoreUpdates = { refresh };
  refresh();
})();
