/*! KagNode Core — native scrolling with accessible overlay rails. */
(() => {
  if (window.KagNodeScroll?.core) return;
  const regions = new Map(),
    root = document.scrollingElement;
  const reduce = matchMedia("(prefers-reduced-motion: reduce)"),
    forced = matchMedia("(forced-colors: active)");
  let frame = 0,
    scanning = 0,
    pendingScans = new Set(),
    serial = 0,
    mode = "custom",
    covered = false;
  try {
    mode =
      localStorage.getItem("kagnode.scrollMode") === "native"
        ? "native"
        : "custom";
  } catch {}
  const active = () => mode === "custom" && !forced.matches;
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  function dimensions(el, axis) {
    const vertical = axis === "y";
    const length =
      el === root
        ? vertical
          ? innerHeight
          : innerWidth
        : vertical
          ? el.clientHeight
          : el.clientWidth;
    const total = vertical ? el.scrollHeight : el.scrollWidth;
    return {
      length,
      total,
      max: Math.max(0, total - length),
      value: vertical ? el.scrollTop : el.scrollLeft,
    };
  }
  function eligible(el, axis) {
    if (
      !(el instanceof HTMLElement) ||
      el.closest("[data-native-scroll],.kn-core-rail")
    )
      return false;
    if (el.closest("[hidden],[inert]")) return false;
    return (
      dimensions(el, axis).max > 1 &&
      (el === root ||
        /auto|scroll/.test(
          getComputedStyle(el)[axis === "y" ? "overflowY" : "overflowX"],
        ))
    );
  }
  function write(state, axis, value) {
    state.el[axis === "y" ? "scrollTop" : "scrollLeft"] = clamp(
      value,
      0,
      dimensions(state.el, axis).max,
    );
    schedule();
  }
  function reveal(state) {
    clearTimeout(state.timer);
    for (const b of Object.values(state.bars)) b.rail.dataset.visible = "true";
    state.timer = setTimeout(() => {
      for (const b of Object.values(state.bars)) delete b.rail.dataset.visible;
    }, 1200);
  }
  function bar(state, axis) {
    const rail = document.createElement("div"),
      thumb = document.createElement("span");
    rail.className = "kn-core-rail kn-core-rail-" + axis;
    thumb.className = "kn-core-thumb";
    rail.append(thumb);
    rail.tabIndex = 0;
    rail.role = "scrollbar";
    rail.setAttribute(
      "aria-orientation",
      axis === "y" ? "vertical" : "horizontal",
    );
    if (!state.el.id) state.el.id = "kn-core-scroll-" + ++serial;
    rail.setAttribute("aria-controls", state.el.id);
    rail.setAttribute("aria-valuemin", "0");
    rail.setAttribute(
      "aria-label",
      (state.el === root
        ? "Page"
        : state.el.getAttribute("aria-label") || "Content") +
        " " +
        (axis === "y" ? "vertical" : "horizontal") +
        " scroll",
    );
    const host = state.el.closest("dialog[open]") || document.body;
    host.append(rail);
    rail.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      rail.focus({ preventScroll: true });
      rail.setPointerCapture(e.pointerId);
      reveal(state);
      const d = dimensions(state.el, axis),
        rect = rail.getBoundingClientRect(),
        length = axis === "y" ? rect.height : rect.width;
      const start = axis === "y" ? e.clientY : e.clientX,
        size = Math.min(length, Math.max(28, (length * d.length) / d.total)),
        travel = Math.max(1, length - size);
      if (e.target !== thumb)
        write(
          state,
          axis,
          ((start - (axis === "y" ? rect.top : rect.left) - size / 2) /
            travel) *
            d.max,
        );
      const initial = dimensions(state.el, axis).value;
      const move = (p) => {
        p.preventDefault();
        write(
          state,
          axis,
          initial +
            (((axis === "y" ? p.clientY : p.clientX) - start) * d.max) / travel,
        );
      };
      const end = () => {
        rail.removeEventListener("pointermove", move);
        rail.removeEventListener("pointerup", end);
        rail.removeEventListener("pointercancel", end);
        rail.removeEventListener("lostpointercapture", end);
        reveal(state);
      };
      rail.addEventListener("pointermove", move);
      rail.addEventListener("pointerup", end);
      rail.addEventListener("pointercancel", end);
      rail.addEventListener("lostpointercapture", end);
    });
    rail.addEventListener("keydown", (e) => {
      const d = dimensions(state.el, axis),
        commands = {
          ArrowDown: 48,
          ArrowRight: 48,
          ArrowUp: -48,
          ArrowLeft: -48,
          PageDown: d.length * 0.85,
          PageUp: -d.length * 0.85,
          Home: -d.max,
          End: d.max,
        };
      if (!(e.key in commands)) return;
      e.preventDefault();
      e.stopPropagation();
      write(state, axis, d.value + commands[e.key]);
      reveal(state);
    });
    return { rail, thumb };
  }
  function mount(el) {
    if (regions.has(el)) return;
    const state = { el, bars: {}, timer: 0 };
    regions.set(el, state);
    resize.observe(el);
    el.classList.add("kn-core-scroll");
    state.listener = () => {
      reveal(state);
      schedule();
    };
    el.addEventListener("scroll", state.listener, { passive: true });
  }
  function remove(el, state) {
    clearTimeout(state.timer);
    resize.unobserve(el);
    el.removeEventListener("scroll", state.listener);
    el.classList.remove("kn-core-scroll");
    for (const b of Object.values(state.bars)) b.rail.remove();
    regions.delete(el);
  }
  function scan(container = document) {
    for (const [el, state] of regions)
      if (!el.isConnected || !active()) remove(el, state);
    if (active()) {
      mount(root);
      const ancestors = [];
      for (
        let parent = container.parentElement;
        parent;
        parent = parent.parentElement
      )
        ancestors.push(parent);
      const nodes = [
        container,
        ...ancestors,
        ...(container.querySelectorAll?.("*") || []),
      ];
      for (const el of nodes)
        if (
          !el.closest?.(".kn-core-rail") &&
          (eligible(el, "y") || eligible(el, "x"))
        )
          mount(el);
    }
    schedule();
  }
  function requestScan(container = document) {
    if (container === document) pendingScans.clear();
    if (!pendingScans.has(document)) pendingScans.add(container);
    if (!scanning)
      scanning = requestAnimationFrame(() => {
        scanning = 0;
        const containers = [...pendingScans];
        pendingScans.clear();
        for (const node of containers) scan(node);
      });
  }
  function schedule() {
    if (!frame) frame = requestAnimationFrame(paint);
  }
  function paint() {
    frame = 0;
    const modal = [...document.querySelectorAll("dialog[open]")].at(-1);
    const loading =
      covered ||
      document.documentElement.classList.contains("kag-loading-covered") ||
      [
        ...document.querySelectorAll(
          ".loading:not([hidden]),.page-loading:not([hidden]),[data-core-loading]",
        ),
      ].some(
        (el) =>
          getComputedStyle(el).position === "fixed" &&
          el.getClientRects().length,
      );
    for (const state of regions.values())
      for (const axis of ["y", "x"]) {
        let b = state.bars[axis];
        const host = state.el.closest("dialog[open]") || document.body;
        if (
          !active() ||
          loading ||
          !eligible(state.el, axis) ||
          (modal && host !== modal)
        ) {
          if (b) b.rail.hidden = true;
          continue;
        }
        if (!b) b = state.bars[axis] = bar(state, axis);
        if (b.rail.parentElement !== host) host.append(b.rail);
        const d = dimensions(state.el, axis),
          r =
            state.el === root
              ? {
                  top: 0,
                  left: 0,
                  right: innerWidth,
                  bottom: innerHeight,
                  width: innerWidth,
                  height: innerHeight,
                }
              : state.el.getBoundingClientRect();
        const clip = {
          top: 0,
          left: 0,
          right: innerWidth,
          bottom: innerHeight,
        };
        for (
          let p = state.el.parentElement;
          p && p !== document.body;
          p = p.parentElement
        ) {
          const css = getComputedStyle(p),
            box = p.getBoundingClientRect();
          if (/auto|scroll|hidden|clip/.test(css.overflowY)) {
            clip.top = Math.max(clip.top, box.top);
            clip.bottom = Math.min(clip.bottom, box.bottom);
          }
          if (/auto|scroll|hidden|clip/.test(css.overflowX)) {
            clip.left = Math.max(clip.left, box.left);
            clip.right = Math.min(clip.right, box.right);
          }
        }
        const top = Math.max(clip.top, r.top) + 3,
          bottom = Math.min(clip.bottom, r.bottom) - 3,
          left = Math.max(clip.left, r.left) + 3,
          right = Math.min(clip.right, r.right) - 3;
        const length = Math.max(0, axis === "y" ? bottom - top : right - left);
        b.rail.hidden = length < 24 || r.width < 1 || r.height < 1;
        const css = getComputedStyle(host),
          offset =
            host !== document.body &&
            (css.transform !== "none" ||
              css.perspective !== "none" ||
              /paint|layout/.test(css.contain))
              ? host.getBoundingClientRect()
              : { top: 0, left: 0 };
        Object.assign(b.rail.style, {
          top: (axis === "y" ? top : bottom - 10) - offset.top + "px",
          left: (axis === "y" ? right - 10 : left) - offset.left + "px",
          [axis === "y" ? "height" : "width"]: length + "px",
        });
        const size = Math.min(
          length,
          Math.max(28, (length * d.length) / d.total),
        );
        b.thumb.style[axis === "y" ? "height" : "width"] = size + "px";
        b.thumb.style.transform = `translate${axis.toUpperCase()}(${(d.value / Math.max(1, d.max)) * (length - size)}px)`;
        b.rail.setAttribute("aria-valuemax", String(Math.round(d.max)));
        b.rail.setAttribute("aria-valuenow", String(Math.round(d.value)));
        b.rail.setAttribute(
          "aria-valuetext",
          Math.round((d.value / Math.max(1, d.max)) * 100) + "%",
        );
      }
  }
  const resize = new ResizeObserver(schedule);
  const observer = new MutationObserver((records) => {
    if (
      records.every(
        (r) =>
          r.target.closest?.(".kn-core-rail") ||
          [...r.addedNodes, ...r.removedNodes].every(
            (n) => n.nodeType !== 1 || n.matches?.(".kn-core-rail"),
          ),
      )
    )
      return;
    for (const record of records) {
      if (!record.target.closest?.(".kn-core-rail")) requestScan(record.target);
    }
  });
  observer.observe(document.body, { childList: true, subtree: true });
  addEventListener("resize", schedule, { passive: true });
  document.addEventListener("scroll", schedule, {
    capture: true,
    passive: true,
  });
  document.addEventListener("kagnode:loading-start", () => {
    covered = true;
    schedule();
  });
  document.addEventListener("kagnode:loading-end", () => {
    covered = false;
    schedule();
  });
  document.addEventListener("transitionend", schedule);
  forced.addEventListener("change", () => scan());
  window.KagNodeScroll = {
    core: true,
    refresh: (container) => requestScan(container || document),
    stop() {},
    get mode() {
      return mode;
    },
    setMode(value) {
      mode = value === "native" ? "native" : "custom";
      try {
        localStorage.setItem("kagnode.scrollMode", mode);
      } catch {}
      scan();
      document.dispatchEvent(
        new CustomEvent("kagnode:scroll-preference", { detail: mode }),
      );
    },
    destroy() {
      observer.disconnect();
      cancelAnimationFrame(scanning);
      cancelAnimationFrame(frame);
      pendingScans.clear();
      resize.disconnect();
      for (const [el, state] of regions) remove(el, state);
    },
  };
  scan();
})();
