/*! KagNode Core — current section and overflow-safe contents navigation. */
(() => {
  if (window.KagNodeToc) return;
  const instances = new Map(),
    reduced = matchMedia("(prefers-reduced-motion: reduce)");
  let frame = 0;
  const section = (link) => {
    try {
      return document.getElementById(decodeURIComponent(link.hash.slice(1)));
    } catch {
      return null;
    }
  };
  function center(list, link) {
    const a = link.getBoundingClientRect(),
      b = list.getBoundingClientRect();
    const top = Math.max(
      0,
      Math.min(
        list.scrollHeight - list.clientHeight,
        list.scrollTop + a.top - b.top + (a.height - list.clientHeight) / 2,
      ),
    );
    if (Math.abs(top - list.scrollTop) > 1)
      list.scrollTo({ top, behavior: reduced.matches ? "instant" : "smooth" });
  }
  function update() {
    frame = 0;
    for (const [toc, state] of instances) {
      if (!toc.isConnected) {
        state.resize.disconnect();
        instances.delete(toc);
        continue;
      }
      const links = [...toc.querySelectorAll('a[href^="#"]')],
        entries = links
          .map((link) => ({ link, target: section(link) }))
          .filter((v) => v.target && v.target.getClientRects().length);
      let current = entries[0];
      const toolbar = document.querySelector(
          ".plan-reading-toolbar,[data-core-reading-toolbar]",
        ),
        padding =
          parseFloat(
            getComputedStyle(document.documentElement).scrollPaddingTop,
          ) || 0,
        margin = entries[0]
          ? parseFloat(getComputedStyle(entries[0].target).scrollMarginTop) || 0
          : 0,
        toolbarOffset = toolbar
          ? Math.max(0, parseFloat(getComputedStyle(toolbar).top) || 0) +
            toolbar.getBoundingClientRect().height +
            24
          : 80,
        offset = Math.max(toolbarOffset, padding + margin + 2);
      for (const entry of entries)
        if (entry.target.getBoundingClientRect().top <= offset) current = entry;
      if (innerHeight + scrollY >= document.documentElement.scrollHeight - 2)
        current = entries.at(-1);
      for (const link of links)
        current?.link === link
          ? link.setAttribute("aria-current", "location")
          : link.removeAttribute("aria-current");
      const list = state.list,
        link = current?.link;
      if (!link) continue;
      const box = link.getBoundingClientRect(),
        parent = list.getBoundingClientRect();
      Object.assign(state.indicator.style, {
        transform: `translate(${box.left - parent.left + list.scrollLeft}px,${box.top - parent.top + list.scrollTop}px)`,
        width: box.width + "px",
        height: box.height + "px",
      });
      toc.classList.add("indicator-ready");
      const size = [
        list.clientWidth,
        list.clientHeight,
        list.scrollHeight,
        link.offsetTop,
      ].join(":");
      if (list.scrollHeight > list.clientHeight + 1) {
        list.tabIndex = 0;
        if (state.current !== link || state.size !== size) {
          const focus =
            list.contains(document.activeElement) &&
            document.activeElement.matches(":focus-visible");
          if (!focus || document.activeElement === link) center(list, link);
        }
      } else list.removeAttribute("tabindex");
      state.current = link;
      state.size = size;
    }
  }
  function refresh() {
    if (!frame) frame = requestAnimationFrame(update);
  }
  function mount() {
    for (const toc of document.querySelectorAll("[data-core-toc],.plan-toc")) {
      if (instances.has(toc)) continue;
      const list =
          toc.querySelector("[data-core-toc-links],.plan-toc-links") || toc,
        indicator = document.createElement("span");
      indicator.className = "plan-toc-indicator";
      indicator.setAttribute("aria-hidden", "true");
      list.append(indicator);
      toc.classList.add("has-indicator");
      const resize = new ResizeObserver(refresh);
      resize.observe(list);
      resize.observe(document.body);
      instances.set(toc, { list, indicator, resize });
      list.addEventListener("focusin", (e) => {
        const link = e.target.closest("a");
        if (link?.matches(":focus-visible")) center(list, link);
      });
      toc.addEventListener("click", (e) => {
        if (toc.classList.contains("plan-toc")) return;
        const link = e.target.closest('a[href^="#"]');
        if (
          !link ||
          e.button ||
          e.metaKey ||
          e.ctrlKey ||
          e.shiftKey ||
          e.altKey
        )
          return;
        const target = section(link);
        if (target) {
          e.preventDefault();
          history.pushState(null, "", link.hash);
          target.scrollIntoView({
            behavior: reduced.matches ? "instant" : "smooth",
            block: "start",
          });
          target.tabIndex = -1;
          target.focus({ preventScroll: true });
        }
      });
    }
    refresh();
  }
  addEventListener("scroll", refresh, { passive: true, capture: true });
  addEventListener("resize", refresh);
  addEventListener("hashchange", refresh);
  document.fonts?.ready.then(refresh);
  let timer;
  new MutationObserver((records) => {
    if (
      records.every(
        (r) =>
          r.type !== "childList" ||
          [...r.addedNodes, ...r.removedNodes].every(
            (n) => n.nodeType !== 1 || n.matches?.(".kn-core-rail"),
          ),
      )
    )
      return;
    clearTimeout(timer);
    timer = setTimeout(mount, 80);
  }).observe(document.body, { childList: true, subtree: true });
  window.KagNodeToc = { refresh, mount };
  mount();
})();
