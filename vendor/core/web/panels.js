/*! KagNode Core — local panel controls and accessible dialog surfaces. */
(() => {
  if (window.KagNodeSegments) return;
  const mounted = new Map();
  let serial = 0;
  const reduce = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
  function mount(host) {
    if (mounted.has(host)) return mounted.get(host);
    const controls = [...host.querySelectorAll("[data-core-segment]")].filter(
      (n) => n.closest("[data-core-segments]") === host,
    );
    if (!controls.length) return;
    const scope = host.closest("[data-core-panels]") || host.parentElement;
    const panels = [...scope.querySelectorAll("[data-core-panel]")].filter(
      (n) =>
        n.closest("[data-core-panels]") ===
          host.closest("[data-core-panels]") ||
        !n.closest("[data-core-panels]"),
    );
    const tabbed = panels.length > 0;
    const indicator = document.createElement("span");
    indicator.className = "kn-core-segment-indicator";
    indicator.setAttribute("aria-hidden", "true");
    host.prepend(indicator);
    host.classList.add("kn-core-segments");
    host.setAttribute("role", tabbed ? "tablist" : "group");
    const available = () =>
      controls.filter(
        (c) =>
          !c.disabled &&
          c.getAttribute("aria-disabled") !== "true" &&
          !c.hidden,
      );
    let current;
    const measure = () => {
      const c = controls.find((n) => n.dataset.coreSegment === current);
      if (!c) return;
      const r = c.getBoundingClientRect(),
        container = host.getBoundingClientRect();
      indicator.style.width = r.width + "px";
      indicator.style.height = r.height + "px";
      indicator.style.transform = `translate(${r.left - container.left - host.clientLeft + host.scrollLeft}px,${r.top - container.top - host.clientTop + host.scrollTop}px)`;
    };
    const select = (
      value,
      { history = false, notify = true, focus = false } = {},
    ) => {
      const control = available().find((c) => c.dataset.coreSegment === value);
      if (!control) return false;
      const previous = current;
      const url = new URL(location.href);
      if (host.dataset.coreUrlParam)
        url.searchParams.set(host.dataset.coreUrlParam, value);
      const event = new CustomEvent("kagnode:segment-change", {
        bubbles: true,
        cancelable: true,
        detail: {
          value,
          previous,
          url:
            control.getAttribute("href") ||
            url.pathname + url.search + url.hash,
        },
      });
      if (notify && previous !== value && !host.dispatchEvent(event))
        return false;
      current = value;
      for (const c of controls) {
        const on = c === control;
        c.dataset.active = String(on);
        c.tabIndex = on ? 0 : -1;
        if (tabbed) c.setAttribute("aria-selected", String(on));
        else if (c.matches("a[href]")) {
          if (on) c.setAttribute("aria-current", "page");
          else c.removeAttribute("aria-current");
        } else c.setAttribute("aria-pressed", String(on));
      }
      for (const p of panels) p.hidden = p.dataset.corePanel !== value;
      if (history && host.dataset.coreUrlParam)
        window.history[history === "push" ? "pushState" : "replaceState"](
          window.history.state,
          "",
          url,
        );
      measure();
      if (focus) {
        control.focus({ preventScroll: true });
        const c = control.getBoundingClientRect(),
          r = host.getBoundingClientRect();
        if (c.left < r.left || c.right > r.right)
          host.scrollBy({
            left: c.left < r.left ? c.left - r.left : c.right - r.right,
            behavior: reduce() ? "auto" : "smooth",
          });
      }
      host.dispatchEvent(
        new CustomEvent("kagnode:segment-selected", {
          bubbles: true,
          detail: { value, previous },
        }),
      );
      window.KagNodeScroll?.refresh(scope);
      return true;
    };
    for (const c of controls) {
      if (tabbed) {
        c.setAttribute("role", "tab");
        c.id ||= "kn-segment-" + ++serial;
        const panel = panels.find(
          (p) => p.dataset.corePanel === c.dataset.coreSegment,
        );
        if (panel) {
          panel.id ||= "kn-panel-" + ++serial;
          panel.setAttribute("role", "tabpanel");
          panel.setAttribute("aria-labelledby", c.id);
          c.setAttribute("aria-controls", panel.id);
        }
      }
      c.addEventListener("click", (event) => {
        if (
          event.button ||
          event.metaKey ||
          event.ctrlKey ||
          event.shiftKey ||
          event.altKey
        )
          return;
        // Real destinations retain native fallback. Authorized partial navigation
        // is owned by a host listening to the cancelable event.
        if (c.matches("a[href]") && !host.dataset.coreUrlParam && !tabbed) {
          if (!select(c.dataset.coreSegment, { notify: true }))
            event.preventDefault();
          return;
        }
        event.preventDefault();
        select(c.dataset.coreSegment, {
          history: host.dataset.coreHistory || "replace",
        });
      });
      c.addEventListener("keydown", (event) => {
        const keys = ["ArrowLeft", "ArrowRight", "Home", "End"];
        if (!keys.includes(event.key)) return;
        const all = available(),
          index = all.indexOf(c),
          rtl = getComputedStyle(host).direction === "rtl";
        const next =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? all.length - 1
              : (index +
                  (event.key === (rtl ? "ArrowLeft" : "ArrowRight") ? 1 : -1) +
                  all.length) %
                all.length;
        event.preventDefault();
        if (c.matches("a[href]") && !tabbed && !host.dataset.coreUrlParam)
          all[next].focus();
        else
          select(all[next].dataset.coreSegment, {
            history: host.dataset.coreHistory || "replace",
            focus: true,
          });
      });
    }
    const syncURL = () =>
      select(
        new URL(location.href).searchParams.get(host.dataset.coreUrlParam) ||
          host.dataset.coreValue ||
          controls.find(
            (c) =>
              c.getAttribute("aria-selected") === "true" ||
              c.getAttribute("aria-current") === "page",
          )?.dataset.coreSegment ||
          available()[0]?.dataset.coreSegment,
        { notify: false },
      );
    const state = { select, measure, syncURL };
    mounted.set(host, state);
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    controls.forEach((c) => observer.observe(c));
    state.observer = observer;
    syncURL();
    document.fonts?.ready.then(measure);
    return state;
  }
  function refresh(container = document) {
    for (const [host, state] of mounted)
      if (!host.isConnected) {
        state.observer.disconnect();
        mounted.delete(host);
      }
    const hosts = [
      ...(container.matches?.("[data-core-segments]") ? [container] : []),
      ...container.querySelectorAll("[data-core-segments]"),
    ];
    for (const host of hosts) {
      const state = mount(host);
      state?.measure();
    }
  }
  window.KagNodeSegments = {
    refresh,
    select: (host, value, options) => mount(host)?.select(value, options),
  };
  addEventListener("popstate", () =>
    document
      .querySelectorAll("[data-core-segments]")
      .forEach((host) => mounted.get(host)?.syncURL()),
  );
  const dialogs = new WeakMap();
  const decorated = new WeakSet();
  const dismissals = new WeakMap();
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || event.defaultPrevented || event.isComposing)
      return;
    const top = [...document.querySelectorAll("dialog[open]")].at(-1);
    if (!top || !dismissals.has(top)) return;
    event.preventDefault();
    dismissals.get(top)();
  });
  function decorate(dialog) {
    if (decorated.has(dialog)) return;
    decorated.add(dialog);
    let header = dialog.querySelector(
      ":scope > header,[data-core-dialog-header]",
    );
    if (!header) {
      header = document.createElement("header");
      const title = dialog.querySelector("h1,h2,h3");
      if (title) header.append(title);
      dialog.prepend(header);
    }
    header.dataset.coreDialogHeader = "";
    header.classList.add("kn-core-dialog-header");
    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.className = "kn-core-dialog-close";
    dismiss.setAttribute("aria-label", "Close dialog");
    dismiss.textContent = "×";
    header.append(dismiss);
    let dirty = false,
      drag,
      confirming = false;
    dialog.addEventListener("input", () => {
      dirty = true;
    });
    dialog.addEventListener("reset", () => {
      dirty = false;
    });
    async function dismissUser() {
      if (confirming || !dialog.open) return;
      const before = new CustomEvent("kagnode:dialog-beforeclose", {
        cancelable: true,
      });
      if (!dialog.dispatchEvent(before)) return;
      if (dirty) {
        const confirmation = document.createElement("dialog");
        confirmation.setAttribute("aria-label", "Discard changes?");
        const heading = document.createElement("h2");
        heading.textContent = "Discard unsaved changes?";
        const keep = document.createElement("button");
        keep.textContent = "Keep editing";
        keep.type = "button";
        const discard = document.createElement("button");
        discard.textContent = "Discard changes";
        discard.type = "button";
        confirmation.append(heading, keep, discard);
        document.body.append(confirmation);
        confirming = true;
        const accepted = await new Promise((resolve) => {
          keep.onclick = () => confirmation.close("keep");
          discard.onclick = () => confirmation.close("discard");
          confirmation.addEventListener(
            "close",
            () => resolve(confirmation.returnValue === "discard"),
            { once: true },
          );
          // Let the dismissal gesture finish before adding another modal to
          // the top layer; its keyup/default must not dismiss the new surface.
          requestAnimationFrame(() => {
            if (!dialog.open) {
              resolve(false);
              return;
            }
            open(confirmation, { initialFocus: keep });
          });
        });
        confirmation.remove();
        confirming = false;
        if (!accepted) return;
      }
      close(dialog);
    }
    dismissals.set(dialog, dismissUser);
    dismiss.addEventListener("click", dismissUser);
    dialog.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.isComposing)
        return;
      // Cancel the keyboard default before opening a nested confirmation;
      // some engines otherwise dismiss the newly opened top-layer dialog.
      event.preventDefault();
      event.stopPropagation();
      dismissUser();
    });
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      dismissUser();
    });
    function clamp(x, y) {
      const viewport = window.visualViewport;
      const left = viewport?.offsetLeft || 0,
        top = viewport?.offsetTop || 0;
      const r = dialog.getBoundingClientRect();
      Object.assign(dialog.style, {
        position: "fixed",
        margin: "0",
        right: "auto",
        bottom: "auto",
        left:
          Math.max(
            left + 8,
            Math.min(x, left + (viewport?.width || innerWidth) - r.width - 8),
          ) + "px",
        top:
          Math.max(
            top + 8,
            Math.min(y, top + (viewport?.height || innerHeight) - r.height - 8),
          ) + "px",
      });
    }
    header.addEventListener("pointerdown", (event) => {
      if (
        innerWidth < 700 ||
        event.button !== 0 ||
        event.target.closest("button,a,input,select,textarea")
      )
        return;
      const r = dialog.getBoundingClientRect();
      drag = {
        id: event.pointerId,
        x: event.clientX,
        y: event.clientY,
        left: r.left,
        top: r.top,
      };
      header.setPointerCapture(event.pointerId);
      event.preventDefault();
    });
    header.addEventListener("pointermove", (event) => {
      if (drag?.id === event.pointerId)
        clamp(
          drag.left + event.clientX - drag.x,
          drag.top + event.clientY - drag.y,
        );
    });
    const finish = () => {
      drag = null;
    };
    header.addEventListener("pointerup", finish);
    header.addEventListener("lostpointercapture", finish);
    const resize = () => {
      if (!dialog.open || !dialog.style.left) return;
      if (innerWidth < 700) {
        for (const name of [
          "position",
          "margin",
          "right",
          "bottom",
          "left",
          "top",
        ])
          dialog.style.removeProperty(name);
        finish();
      } else {
        const r = dialog.getBoundingClientRect();
        clamp(r.left, r.top);
      }
    };
    dialog.addEventListener("close", () => {
      dirty = false;
      finish();
      window.removeEventListener("resize", resize);
      window.visualViewport?.removeEventListener("resize", resize);
    });
    dialog.addEventListener("kagnode:dialog-mounted", () => {
      window.addEventListener("resize", resize);
      window.visualViewport?.addEventListener("resize", resize);
    });
  }
  function open(
    dialog,
    { sheet = false, returnFocus = document.activeElement, initialFocus } = {},
  ) {
    if (!(dialog instanceof HTMLDialogElement))
      throw Error("A native dialog is required");
    if (dialog.open) return dialog;
    for (const name of ["position", "margin", "right", "bottom", "left", "top"])
      dialog.style.removeProperty(name);
    if (sheet) dialog.dataset.coreSheet = "";
    dialog.classList.add("kn-core-dialog");
    decorate(dialog);
    window.KagNodeCore?.navigation.close();
    window.KagNodeCore?.closeAccount();
    dialogs.set(dialog, { returnFocus });
    dialog.showModal();
    dialog.dispatchEvent(new Event("kagnode:dialog-mounted"));
    if (!reduce())
      dialog.animate(
        [
          {
            opacity: 0.65,
            transform: sheet ? "translateY(16px)" : "translateY(4px)",
          },
          { opacity: 1, transform: "none" },
        ],
        { duration: 160, easing: "ease-out" },
      );
    (
      initialFocus ||
      dialog.querySelector('[autofocus],button,input,a[href],[tabindex="0"]')
    )?.focus({ preventScroll: true });
    document.dispatchEvent(
      new CustomEvent("kagnode:dialog-open", { detail: { dialog } }),
    );
    window.KagNodeScroll?.refresh(dialog);
    return dialog;
  }
  function close(dialog, result = "") {
    if (dialog?.open) dialog.close(result);
  }
  document.addEventListener("click", (event) => {
    const dismiss = event.target.closest?.("[data-core-dismiss]");
    if (dismiss) {
      const d = dismiss.closest("dialog");
      if (d) {
        event.preventDefault();
        dismissals.has(d)
          ? dismissals.get(d)()
          : close(d, dismiss.dataset.coreDismiss);
      }
    }
    const trigger = event.target.closest?.("[data-core-dialog-open]");
    if (trigger) {
      const d = document.getElementById(trigger.dataset.coreDialogOpen);
      if (d) {
        event.preventDefault();
        open(d, { sheet: d.hasAttribute("data-core-sheet") });
      }
    }
  });
  document.addEventListener(
    "close",
    (event) => {
      if (
        !(event.target instanceof HTMLDialogElement) ||
        event.target.open ||
        !dialogs.has(event.target)
      )
        return;
      const state = dialogs.get(event.target);
      dialogs.delete(event.target);
      if (
        state?.returnFocus?.isConnected &&
        !state.returnFocus.closest("[inert],[hidden]")
      )
        state.returnFocus.focus({ preventScroll: true });
      else
        document
          .querySelector("[data-core-mobile]")
          ?.focus({ preventScroll: true });
      document.dispatchEvent(
        new CustomEvent("kagnode:dialog-close", {
          detail: { dialog: event.target },
        }),
      );
      window.KagNodeScroll?.refresh();
    },
    true,
  );
  window.KagNodeDialogs = {
    open,
    close,
    dismiss: (dialog) => dismissals.get(dialog)?.(),
  };
  refresh();
  document.addEventListener("kagnode:account-ready", () => refresh());
})();
