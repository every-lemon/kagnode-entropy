/*! Browser-to-KagNode appearance only. No profile, vault or server credentials. */
(() => {
  const core = window.KagNodeCore;
  if (!core || window.KagNodeAppearance) return;
  const config = core.config,
    issuer = new URL(config.issuer || "https://nebulacoder.com").origin;
  let session = window.KagNodeTheme.session,
    token = "",
    expires = 0,
    revision = 0,
    current = "system",
    pending,
    popup,
    refreshTask;
  const key = () => "kagnode.core.appearance." + session.subject;
  const read = () => {
    try {
      return JSON.parse(localStorage.getItem(key()) || "null");
    } catch {
      return null;
    }
  };
  const write = (value) => {
    try {
      value
        ? localStorage.setItem(key(), JSON.stringify(value))
        : localStorage.removeItem(key());
    } catch {}
  };
  const b64 = (bytes) =>
    btoa(String.fromCharCode(...bytes))
      .replaceAll("+", "-")
      .replaceAll("/", "_")
      .replaceAll("=", "");
  const channel =
    typeof BroadcastChannel === "function"
      ? new BroadcastChannel("kagnode-core-appearance")
      : null;
  let generation = 0;
  const native = config.officialAccount === true || location.origin === issuer;
  function apply(value) {
    if (!["system", "light", "dark"].includes(value.theme)) return;
    if (!session.authenticated) return;
    current = value.theme;
    revision = value.version;
    window.KagNodeTheme.apply(current);
    document
      .querySelectorAll("[data-core-theme]")
      .forEach((b) =>
        b.setAttribute("aria-pressed", String(b.dataset.coreTheme === current)),
      );
  }
  async function renew() {
    const epoch = generation;
    if (token && Date.now() < expires - 30000) return token;
    const grant = read();
    if (!grant?.refresh_token) return "";
    if (refreshTask) return refreshTask;
    const run = async () => {
      if (token && Date.now() < expires - 30000) return token;
      const latest = read();
      if (!latest?.refresh_token) return "";
      const r = await fetch(issuer + "/api/v1/appearance/token", {
        method: "POST",
        credentials: "omit",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          grant_type: "refresh_token",
          refresh_token: latest.refresh_token,
        }),
      });
      if (epoch !== generation) return "";
      if (!r.ok) {
        if ([400, 401, 403].includes(r.status)) {
          write(null);
          token = "";
        }
        throw Error("Reconnect your KagNode appearance preferences.");
      }
      const value = await r.json();
      if (epoch !== generation) return "";
      token = value.access_token;
      expires = Date.now() + value.expires_in * 1000;
      write({
        refresh_token: value.refresh_token,
        preference: latest.preference,
      });
      channel?.postMessage({
        type: "token",
        subject: session.subject,
        token,
        expires,
      });
      return token;
    };
    refreshTask = (
      navigator.locks
        ? navigator.locks.request("kagnode-appearance-refresh", run)
        : run()
    ).finally(() => (refreshTask = null));
    return refreshTask;
  }
  async function request(method = "GET", patch, retry = true) {
    const epoch = generation;
    const headers = { Accept: "application/json" };
    let url, credentials;
    if (native) {
      url = "/api/account/appearance";
      credentials = "same-origin";
      if (patch)
        headers["X-CSRF-Token"] =
          document.querySelector("input[name=_csrf]")?.value || "";
    } else {
      const access = await renew();
      if (epoch !== generation) return null;
      if (!access) return null;
      headers.Authorization = "Bearer " + access;
      url = issuer + "/api/v1/appearance/preferences";
      credentials = "omit";
    }
    if (patch) headers["Content-Type"] = "application/json";
    const r = await fetch(url, {
      method: native && method === "PATCH" ? "POST" : method,
      headers,
      credentials,
      ...(patch ? { body: JSON.stringify(patch) } : {}),
    });
    if (epoch !== generation) return null;
    if (r.status === 401 && !native && retry) {
      token = "";
      expires = 0;
      return request(method, patch, false);
    }
    if (r.status === 409) {
      await sync();
      throw Error("Appearance changed elsewhere. Review it and try again.");
    }
    if (!r.ok || r.redirected)
      throw Error(
        "Appearance could not sync. Your local account remains available.",
      );
    const value = await r.json();
    if (epoch !== generation) return null;
    if (value.theme === null) value.theme = "system";
    apply(value);
    const grant = read();
    if (grant) write({ ...grant, preference: value });
    return value;
  }
  async function sync() {
    if (!session.authenticated) return;
    try {
      await request();
    } catch {
      /* Keep the last valid preference offline. */
    }
  }
  async function connect() {
    if (!session.authenticated) return;
    if (native) return sync();
    const verifier = b64(crypto.getRandomValues(new Uint8Array(48))),
      state = b64(crypto.getRandomValues(new Uint8Array(24)));
    const challenge = b64(
      new Uint8Array(
        await crypto.subtle.digest(
          "SHA-256",
          new TextEncoder().encode(verifier),
        ),
      ),
    );
    pending = { verifier, state, subject: session.subject };
    const url = new URL("/auth/appearance/authorize", issuer);
    url.search = new URLSearchParams({
      origin: location.origin,
      code_challenge: challenge,
      state,
      app: config.app || "KagNode application",
    });
    popup = open(
      url,
      "kagnode-appearance-connect",
      "popup,width=540,height=700",
    );
    if (!popup) throw Error("Allow the account connection window to open.");
  }
  addEventListener("message", async (e) => {
    if (
      e.origin !== issuer ||
      e.source !== popup ||
      !pending ||
      e.data?.type !== "kagnode-appearance-code" ||
      e.data.state !== pending.state ||
      session.subject !== pending.subject
    )
      return;
    const attempt = pending;
    pending = null;
    try {
      const r = await fetch(issuer + "/api/v1/appearance/token", {
        method: "POST",
        credentials: "omit",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          grant_type: "authorization_code",
          code: e.data.code,
          code_verifier: attempt.verifier,
        }),
      });
      if (!r.ok) throw Error("Connection expired. Try connecting again.");
      const value = await r.json();
      if (session.subject !== attempt.subject || !session.authenticated) return;
      token = value.access_token;
      expires = Date.now() + value.expires_in * 1000;
      write({ refresh_token: value.refresh_token });
      await sync();
      render();
    } catch (error) {
      status(error.message);
    }
  });
  function status(message) {
    document
      .querySelectorAll("[data-core-preference-status]")
      .forEach((el) => (el.textContent = message));
  }
  async function choose(theme) {
    try {
      if (!native && !read()) {
        window.KagNodeTheme.saveLocal(theme);
        current = theme;
        render();
        return;
      }
      await request("PATCH", { theme, version: revision });
      channel?.postMessage({ subject: session.subject });
      status("Appearance saved.");
    } catch (error) {
      status(error.message);
    }
  }
  async function disconnect() {
    const grant = read();
    if (grant) {
      try {
        await fetch(issuer + "/api/v1/appearance/revoke", {
          method: "POST",
          credentials: "omit",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ refresh_token: grant.refresh_token }),
        });
      } catch {}
    }
    write(null);
    token = "";
    expires = 0;
    render();
  }
  function render() {
    if (!session.authenticated) return;
    const host =
      document.querySelector(
        "[data-core-preferences-host],.kn-account-settings",
      ) ||
      (/(?:settings|preferences|account)$/.test(location.pathname)
        ? document.querySelector(
            "#settings-view,.account-content,#main-content,.content,main",
          )
        : null);
    if (!host || host.closest("[hidden]")) return;
    let panel = host.querySelector("[data-core-preferences-panel]");
    if (!panel) {
      panel = document.createElement("section");
      panel.className = "kn-core-preferences";
      panel.dataset.corePreferencesPanel = "";
      panel.innerHTML =
        '<h3>Appearance</h3><p>Choose how KagNode looks on this device.</p><div class="kn-core-preference-options"><button type="button" data-core-theme="system">System</button><button type="button" data-core-theme="light">Light</button><button type="button" data-core-theme="dark">Dark</button></div><div class="kn-core-preference-options"><button type="button" data-core-appearance-connect>Sync with KagNode Account</button><button type="button" data-core-appearance-disconnect hidden>Disconnect appearance sync</button></div><h3>Scrolling</h3><div class="kn-core-preference-options"><button type="button" data-core-scroll-mode="custom">KagNode rails</button><button type="button" data-core-scroll-mode="native">Native scrolling</button></div><p role="status" data-core-preference-status></p>';
      host.append(panel);
    }
    panel
      .querySelectorAll("[data-core-theme]")
      .forEach((b) =>
        b.setAttribute(
          "aria-pressed",
          String(
            b.dataset.coreTheme ===
              (document.documentElement.dataset.coreThemePreference || current),
          ),
        ),
      );
    panel
      .querySelectorAll("[data-core-scroll-mode]")
      .forEach((b) =>
        b.setAttribute(
          "aria-pressed",
          String(b.dataset.coreScrollMode === window.KagNodeScroll?.mode),
        ),
      );
    panel.querySelector("[data-core-appearance-connect]").hidden =
      native || !!read();
    panel.querySelector("[data-core-appearance-disconnect]").hidden =
      native || !read();
    if (
      native &&
      location.origin === issuer &&
      !panel.querySelector("[data-core-appearance-connections]")
    ) {
      const list = document.createElement("div");
      list.dataset.coreAppearanceConnections = "";
      panel.append(list);
      fetch("/account/appearance/connections", {
        credentials: "same-origin",
        headers: { Accept: "application/json" },
      })
        .then((r) => (r.ok ? r.json() : null))
        .then((value) => {
          if (!value?.connections?.length) return;
          const title = document.createElement("h3");
          title.textContent = "Connected appearance apps";
          list.append(title);
          for (const grant of value.connections.filter((g) => !g.revoked)) {
            const row = document.createElement("div"),
              label = document.createElement("span"),
              button = document.createElement("button");
            label.textContent = grant.origin;
            button.textContent = "Disconnect";
            button.type = "button";
            row.className = "kn-core-preference-options";
            row.append(label, button);
            list.append(row);
            button.onclick = async () => {
              button.disabled = true;
              const r = await fetch(
                "/account/appearance/connections/" +
                  encodeURIComponent(grant.id) +
                  "/revoke",
                {
                  method: "POST",
                  credentials: "same-origin",
                  headers: {
                    "X-CSRF-Token":
                      document.querySelector("input[name=_csrf]")?.value || "",
                  },
                },
              );
              if (r.ok) row.remove();
              else {
                button.disabled = false;
                status("Connection could not be revoked. Try again.");
              }
            };
          }
        })
        .catch(() => {});
    }
  }
  document.addEventListener("click", (e) => {
    const target = e.target.closest(
      "[data-core-theme],[data-core-appearance-connect],[data-core-appearance-disconnect],[data-core-scroll-mode]",
    );
    if (!target) return;
    if (target.dataset.coreTheme) choose(target.dataset.coreTheme);
    else if (target.hasAttribute("data-core-appearance-connect"))
      connect().catch((error) => status(error.message));
    else if (target.hasAttribute("data-core-appearance-disconnect"))
      disconnect();
    else {
      window.KagNodeScroll?.setMode(target.dataset.coreScrollMode);
      render();
    }
  });
  document.addEventListener("kagnode:core-session", (e) => {
    generation++;
    pending = null;
    popup?.close();
    refreshTask = null;
    session = {
      authenticated: !!e.detail.authenticated,
      subject: String(e.detail.subject || ""),
    };
    token = "";
    expires = 0;
    if (session.authenticated) {
      const value = read()?.preference;
      if (value) apply(value);
      sync();
      render();
    }
  });
  addEventListener("focus", () => {
    sync();
    render();
  });
  channel?.addEventListener("message", (e) => {
    if (e.data.subject !== session.subject) return;
    if (e.data.type === "token") {
      token = e.data.token;
      expires = e.data.expires;
    } else sync();
  });
  let rendering = false;
  new MutationObserver(() => {
    if (!rendering) {
      rendering = true;
      requestAnimationFrame(() => {
        rendering = false;
        render();
      });
    }
  }).observe(document.body, { childList: true, subtree: true });
  setInterval(sync, 60000);
  window.KagNodeAppearance = { connect, disconnect, sync, render };
  if (session.authenticated) {
    sync();
    render();
  }
})();
