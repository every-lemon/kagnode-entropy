/*! KagNode Core — authenticated, host-opted-in summaries without admin data. */
(() => {
  if (window.KagNodeWhatsNew) return;
  const dirty = new Set(),
    seen = new Set();
  let generation = 0,
    pending = null,
    panel = null,
    controller = null,
    reading = false,
    checked = 0;
  const session = () => window.KagNodeTheme?.session;
  const ready = () =>
    !!session()?.authenticated &&
    (window.KagNodeWhatsNewHost?.ready?.() === true ||
      document.body.dataset.coreNotices === "enabled");
  const editing = () => {
    for (const form of dirty) if (!form.isConnected) dirty.delete(form);
    return (
      dirty.size > 0 ||
      document.activeElement?.matches(
        'input,textarea,select,[contenteditable="true"]',
      ) ||
      !!document.querySelector(
        'dialog[open],[data-core-account-trigger][aria-expanded="true"]',
      ) ||
      document.documentElement.classList.contains("kag-loading-covered")
    );
  };
  function notice(value) {
    const n = value?.notice,
      text = (s, max) =>
        typeof s === "string" &&
        s.length > 0 &&
        s.length <= max &&
        !/[\x00-\x1f<>]/.test(s);
    return value?.available === true &&
      n &&
      Object.keys(n).sort().join(",") === "id,items,title,version" &&
      /^[a-f0-9]{64}$/.test(n.id) &&
      text(n.version, 40) &&
      /^\d+\.\d+\.\d+$/.test(n.version) &&
      text(n.title, 120) &&
      Array.isArray(n.items) &&
      n.items.length >= 1 &&
      n.items.length <= 8 &&
      n.items.every((s) => text(s, 240))
      ? n
      : null;
  }
  async function dismiss(n, epoch) {
    if (epoch !== generation || !ready()) return;
    const headers = (await window.KagNodeWhatsNewHost?.headers?.()) || {
      "X-CSRF-Token":
        document.querySelector('meta[name="csrf-token"]')?.content ||
        document.querySelector('input[name="_csrf"],input[name="csrf"]')
          ?.value ||
        "",
    };
    if (epoch !== generation || !ready()) return;
    const response = await fetch("/api/v1/kagnode-core/whats-new/dismiss", {
      method: "POST",
      credentials: "same-origin",
      cache: "no-store",
      headers: { ...headers, "Content-Type": "application/json" },
      body: JSON.stringify({ id: n.id }),
    });
    if (!response.ok)
      throw Error("Unable to remember this preference. Try again.");
  }
  function show() {
    if (
      !pending ||
      panel ||
      !ready() ||
      editing() ||
      seen.has(session().subject + ":" + pending.id)
    )
      return;
    const n = pending,
      epoch = generation;
    pending = null;
    seen.add(session().subject + ":" + n.id);
    const d = document.createElement("dialog");
    panel = d;
    d.setAttribute("aria-label", "What's new");
    d.dataset.coreSheet = "";
    const heading = document.createElement("h2");
    heading.textContent = n.title;
    const version = document.createElement("p");
    version.textContent = "UI update · " + n.version;
    const list = document.createElement("ul");
    for (const text of n.items) {
      const item = document.createElement("li");
      item.textContent = text;
      list.append(item);
    }
    const status = document.createElement("p");
    status.setAttribute("role", "status");
    const close = document.createElement("button");
    close.type = "button";
    close.textContent = "Got it";
    let saved = false,
      saving = false;
    close.onclick = async () => {
      if (saving) return;
      saving = true;
      close.disabled = true;
      try {
        await dismiss(n, epoch);
        if (epoch !== generation) return;
        saved = true;
        window.KagNodeDialogs.close(d);
      } catch {
        status.textContent = "Unable to remember this preference. Try again.";
      } finally {
        saving = false;
        close.disabled = false;
      }
    };
    d.append(heading, version, list, status, close);
    document.body.append(d);
    d.addEventListener(
      "close",
      () => {
        panel = null;
        d.remove();
        if (!saved && epoch === generation && ready())
          dismiss(n, epoch).catch(() => {});
      },
      { once: true },
    );
    window.KagNodeDialogs.open(d, { sheet: true });
  }
  async function refresh() {
    if (!ready() || reading || Date.now() - checked < 30000) {
      show();
      return;
    }
    const epoch = generation;
    reading = true;
    controller = new AbortController();
    try {
      const r = await fetch("/api/v1/kagnode-core/whats-new", {
        credentials: "same-origin",
        cache: "no-store",
        signal: controller.signal,
      });
      if (!r.ok) return;
      const value = await r.json();
      if (epoch !== generation || !ready()) return;
      pending = notice(value);
      show();
    } catch {
    } finally {
      if (epoch === generation) {
        reading = false;
        checked = Date.now();
      }
    }
  }
  document.addEventListener("input", (event) => {
    const form = event.target.closest?.("form");
    if (form) dirty.add(form);
  });
  document.addEventListener("reset", (event) => {
    dirty.delete(event.target);
    setTimeout(show, 0);
  });
  document.addEventListener("kagnode:form-saved", (event) => {
    const form = event.detail?.form || event.target.closest?.("form");
    if (form) dirty.delete(form);
    show();
  });
  document.addEventListener("kagnode:core-session", () => {
    generation++;
    controller?.abort();
    reading = false;
    checked = 0;
    pending = null;
    if (panel) window.KagNodeDialogs.close(panel);
    refresh();
  });
  document.addEventListener("kagnode:account-ready", refresh);
  for (const type of [
    "focusout",
    "kagnode:dialog-close",
    "kagnode:loading-end",
  ])
    document.addEventListener(type, () => setTimeout(show, 0));
  addEventListener("focus", refresh);
  setInterval(() => {
    if (!document.hidden) refresh();
  }, 30000);
  window.KagNodeWhatsNew = { refresh };
  refresh();
})();
