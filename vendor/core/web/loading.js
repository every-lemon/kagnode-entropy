/*! KagNode Core — split-orbit loading mark. AGPL-3.0-only. */
(() => {
  if (window.KagNodeLoading) return;
  const config = JSON.parse(
    document.querySelector('meta[name="kagnode-core"]')?.content || "{}",
  );
  function spinner() {
    const frame = document.createElement("span");
    frame.className = "kn-core-spinner";
    frame.setAttribute("aria-hidden", "true");
    // Static, trusted geometry. The canonical mark stays still and readable
    // while its separated accent travels around the outside.
    frame.innerHTML =
      '<span class="kn-core-spinner-layer"><svg class="kn-core-spinner-orbit" viewBox="0 0 96 96" fill="none" focusable="false" aria-hidden="true">' +
      '<circle class="kn-core-spinner-track" cx="48" cy="48" r="39"/>' +
      '<g class="kn-core-spinner-rotor"><path class="kn-core-spinner-sweep" d="M14.23 67.5A39 39 0 0 1 67.5 14.23"/>' +
      '<path class="kn-core-spinner-node" d="m83 39 6 7-3 10-6-6Z"/></g></svg></span>';
    const mark = document.createElement("img");
    mark.className = "kn-core-spinner-mark";
    mark.src = config.mark || (config.base || "") + "mark.svg";
    mark.alt = "";
    mark.width = mark.height = 48;
    mark.draggable = false;
    frame.append(mark);
    return frame;
  }
  function enhance() {
    document
      .querySelectorAll(
        "[data-core-loading],#loading.loading,#loading.loading-screen," +
          "[data-loading-screen].loading-screen," +
          "#kagnode-page-loading > .kag-loading-content",
      )
      .forEach((host) => {
        if (!host.querySelector(":scope > .kn-core-spinner"))
          host.prepend(spinner());
        host.setAttribute("data-core-loading", "");
      });
  }
  const operations = new Map(),
    inline = new Map();
  let serial = 0,
    overlay,
    topbar,
    legacy,
    covered = false,
    previousPending = -1;
  const node = (tag, cls) => {
    const n = document.createElement(tag);
    n.className = cls;
    return n;
  };
  function render() {
    const all = [...operations.values()],
      active = all.filter((n) => n.mode === "overlay").at(-1);
    if (!overlay) {
      overlay = node("div", "kn-core-loading");
      overlay.id = "kn-core-loading";
      overlay.setAttribute("role", "status");
      overlay.setAttribute("aria-live", "polite");
      overlay.append(spinner(), node("span", "kn-core-loading-label"));
    }
    if (overlay) {
      const parent =
        [...document.querySelectorAll("dialog[open]")].at(-1) || document.body;
      if (overlay.parentElement !== parent) parent.append(overlay);
      if (active) {
        overlay.querySelector(".kn-core-loading-label").textContent =
          active.label;
      }
      overlay.hidden = !active;
      overlay.setAttribute("aria-hidden", String(!active));
      if (!overlay.isConnected) document.body.append(overlay);
    }
    if (!topbar) {
      topbar = node("div", "kn-core-loading-topbar");
      topbar.setAttribute("aria-hidden", "true");
      topbar.append(node("span", "kn-core-loading-runner"));
      document.body.append(topbar);
    }
    if (topbar)
      topbar.hidden = !all.some(
        (n) => n.mode === "topbar" || n.mode === "overlay",
      );
    document.documentElement.classList.toggle("kag-loading-covered", !!active);
    if (covered !== !!active) {
      covered = !!active;
      document.dispatchEvent(
        new Event(covered ? "kagnode:loading-start" : "kagnode:loading-end"),
      );
    }
    if (previousPending !== operations.size) {
      previousPending = operations.size;
      document.dispatchEvent(
        new CustomEvent("kagnode:loading-operation", {
          detail: { pending: operations.size },
        }),
      );
    }
    const targets = new Set(
      all.filter((n) => n.mode === "inline").map((n) => n.target),
    );
    for (const [target, state] of inline)
      if (!targets.has(target)) {
        state.node.remove();
        if (state.busy === null) target.removeAttribute("aria-busy");
        else target.setAttribute("aria-busy", state.busy);
        inline.delete(target);
      }
    for (const target of targets) {
      let state = inline.get(target);
      if (!state) {
        const n = node("span", "kn-core-inline-progress");
        n.setAttribute("role", "status");
        n.append(spinner(), node("span", "kn-core-inline-label"));
        state = { node: n, busy: target.getAttribute("aria-busy") };
        inline.set(target, state);
        target.append(n);
        target.setAttribute("aria-busy", "true");
      }
      state.node.querySelector(".kn-core-inline-label").textContent = all
        .filter((n) => n.target === target)
        .at(-1).label;
    }
  }
  function begin({ label = "Loading…", mode = "topbar", target } = {}) {
    if (
      !["overlay", "topbar", "inline"].includes(mode) ||
      (mode === "inline" && !(target instanceof HTMLElement))
    )
      throw Error("Choose a valid loading surface");
    const id = ++serial,
      state = { label: String(label).slice(0, 200), mode, target };
    operations.set(id, state);
    render();
    const finish = () => {
      if (operations.delete(id)) render();
    };
    finish.update = finish.phase = (value) => {
      if (operations.has(id)) {
        state.label = String(value).slice(0, 200);
        render();
      }
      return finish;
    };
    return finish;
  }
  render();
  document.addEventListener("close", render, true);
  document.addEventListener("kagnode:dialog-open", render);
  window.KagNodeLoading = {
    spinner,
    enhance,
    begin,
    show(label = "Loading…") {
      legacy?.();
      legacy = begin({ label, mode: "overlay" });
    },
    hide() {
      legacy?.();
      legacy = null;
    },
    get pending() {
      return operations.size;
    },
  };
})();
