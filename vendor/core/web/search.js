/*! KagNode Core — scoped, accessible quick search. AGPL-3.0-only. */
(() => {
  if (window.KagNodeSearch) return;
  let host,
    dialog,
    input,
    list,
    status,
    more,
    retry,
    trigger,
    endpoint,
    shortcuts = [],
    rows = [],
    index = -1,
    request,
    timer,
    sequence = 0;
  const local = (value) => {
    if (typeof value !== "string" || !value) return null;
    try {
      const u = new URL(value, location.href);
      return u.origin === location.origin &&
        ["http:", "https:"].includes(u.protocol)
        ? u.pathname + u.search + u.hash
        : null;
    } catch {
      return null;
    }
  };
  const element = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  };
  function active(value) {
    index = rows.length ? (value + rows.length) % rows.length : -1;
    [...list.children].forEach((e, i) =>
      e.setAttribute("aria-selected", String(i === index)),
    );
    if (index >= 0) {
      input.setAttribute("aria-activedescendant", list.children[index].id);
      list.children[index].scrollIntoView({ block: "nearest" });
    } else input.removeAttribute("aria-activedescendant");
  }
  function render(values, message = "") {
    rows = values;
    index = -1;
    input.removeAttribute("aria-activedescendant");
    list.replaceChildren();
    values.forEach((value, i) => {
      const a = element("a", "kn-core-search-row");
      a.href = value.url;
      a.id = "kn-search-result-" + i;
      a.role = "option";
      a.tabIndex = -1;
      a.setAttribute("aria-selected", "false");
      const copy = element("span", "kn-core-search-copy");
      copy.append(
        element("strong", "", value.title),
        element("small", "", value.description || ""),
      );
      a.append(
        copy,
        element("span", "kn-core-search-type", value.type || "Page"),
      );
      a.addEventListener("click", () => dialog.close());
      list.append(a);
    });
    status.textContent =
      message ||
      (!values.length
        ? "No matching results. Try another phrase."
        : values.length + " results");
    window.KagNodeScroll?.refresh();
  }
  function cancel() {
    sequence++;
    clearTimeout(timer);
    request?.abort();
    input?.removeAttribute("aria-busy");
  }
  async function search() {
    const term = input.value.trim(),
      intent = sequence,
      matching = shortcuts.filter(
        (v) =>
          !term ||
          (v.title + " " + v.description)
            .toLowerCase()
            .includes(term.toLowerCase()),
      );
    retry.hidden = true;
    more.href = "/search?q=" + encodeURIComponent(term);
    more.hidden = !term || !endpoint;
    if (!term || !endpoint) {
      render(matching);
      return;
    }
    render(matching, "Searching…");
    input.setAttribute("aria-busy", "true");
    const controller = new AbortController();
    request = controller;
    const timeout = setTimeout(() => controller.abort(), 8000);
    try {
      const url = new URL(endpoint, location.origin);
      url.searchParams.set("q", term);
      const response = await fetch(url.pathname + url.search, {
        headers: { Accept: "application/json" },
        redirect: "error",
        signal: controller.signal,
      });
      if (
        !response.ok ||
        response.redirected ||
        !response.headers.get("content-type")?.includes("application/json")
      )
        throw Error("Search unavailable");
      const data = await response.json();
      if (intent !== sequence || !dialog.open) return;
      const results = Array.isArray(data.results)
        ? data.results
            .filter((v) => v && typeof v.title === "string" && local(v.url))
            .map((v) => ({ ...v, url: local(v.url) }))
        : [];
      render([...matching, ...results].slice(0, 20));
      if (local(data.moreUrl)) more.href = local(data.moreUrl);
    } catch (error) {
      if (intent !== sequence || !dialog.open) return;
      render(matching, "Search could not load. Try again or open full search.");
      retry.hidden = false;
    } finally {
      clearTimeout(timeout);
      if (intent === sequence) input.removeAttribute("aria-busy");
    }
  }
  function open(source) {
    if (!dialog) return;
    if (dialog.open) {
      input.focus();
      return;
    }
    trigger = source || document.activeElement;
    window.KagNodeCore?.closeAccount();
    window.KagNodeCore?.navigation.close();
    cancel();
    input.value = "";
    dialog.showModal();
    input.focus({ preventScroll: true });
    search();
  }
  function mount() {
    host = document.querySelector("[data-core-search-source]");
    if (!host) return;
    endpoint = local(host.dataset.endpoint || "");
    shortcuts = [...host.querySelectorAll("a[href]")]
      .map((a) => ({
        title: a.dataset.title || a.textContent.trim(),
        description: a.dataset.description || "",
        url: local(a.getAttribute("href")),
        type: "Page",
      }))
      .filter((v) => v.url);
    dialog = element("dialog", "kn-core-search");
    dialog.id = "command-palette";
    dialog.setAttribute("aria-label", "Search KagNode");
    const head = element("div", "kn-core-search-head"),
      label = element("label", "kn-core-search-label", "Search KagNode");
    input = element("input", "kn-core-search-input");
    input.id = "command-query";
    input.type = "search";
    input.maxLength = 200;
    input.autocomplete = "off";
    input.placeholder = host.dataset.placeholder || "Search KagNode…";
    input.role = "combobox";
    input.setAttribute("aria-controls", "kn-core-search-results");
    input.setAttribute("aria-expanded", "true");
    input.setAttribute("aria-autocomplete", "list");
    label.htmlFor = input.id;
    const close = element("button", "kn-core-search-close", "Esc");
    close.type = "button";
    close.setAttribute("aria-label", "Close quick search");
    close.addEventListener("click", () => dialog.close());
    head.append(label, input, close);
    list = element("div", "kn-core-search-results");
    list.id = "kn-core-search-results";
    list.role = "listbox";
    list.setAttribute("aria-label", "Search results");
    status = element("p", "kn-core-search-status");
    status.role = "status";
    status.setAttribute("aria-live", "polite");
    const footer = element("div", "kn-core-search-footer");
    more = element("a", "", "Open full search");
    retry = element("button", "", "Try again");
    retry.type = "button";
    retry.hidden = true;
    retry.addEventListener("click", () => {
      cancel();
      search();
    });
    footer.append(status, retry, more);
    dialog.append(head, list, footer);
    document.body.append(dialog);
    const viewport = () => {
      const height = window.visualViewport?.height || innerHeight;
      dialog.style.setProperty(
        "--kn-search-viewport",
        Math.max(120, height - 24) + "px",
      );
      dialog.style.top = (window.visualViewport?.offsetTop || 0) + "px";
      dialog.style.marginTop = Math.min(80, Math.max(12, height * 0.08)) + "px";
    };
    window.visualViewport?.addEventListener("resize", viewport);
    window.visualViewport?.addEventListener("scroll", viewport);
    addEventListener("resize", viewport);
    viewport();

    input.addEventListener("input", () => {
      cancel();
      const term = input.value.trim();
      // Invalidate both selection and navigation before the debounce delay.
      // Enter must never use the results or full-search URL of an old query.
      more.href = "/search?q=" + encodeURIComponent(term);
      more.hidden = !term || !endpoint;
      retry.hidden = true;
      render([], term ? "Searching…" : "");
      if (!term) {
        search();
        return;
      }
      timer = setTimeout(search, 150);
    });
    input.addEventListener("keydown", (e) => {
      if (e.isComposing || e.keyCode === 229) return;
      if (["ArrowDown", "ArrowUp"].includes(e.key)) {
        e.preventDefault();
        active(
          index < 0 && e.key === "ArrowUp"
            ? rows.length - 1
            : index + (e.key === "ArrowDown" ? 1 : -1),
        );
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (rows[index]) location.assign(rows[index].url);
        else if (!more.hidden) location.assign(more.href);
      }
    });
    dialog.addEventListener("keydown", (e) => {
      if (e.isComposing || e.keyCode === 229) {
        // WebKit can still run the dialog's native Escape dismissal even
        // when our close handler returns during composition.
        if (e.key === "Escape") e.preventDefault();
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        dialog.close();
      }
    });
    dialog.addEventListener("click", (e) => {
      if (e.target === dialog) {
        const r = dialog.getBoundingClientRect();
        if (
          e.clientX < r.left ||
          e.clientX > r.right ||
          e.clientY < r.top ||
          e.clientY > r.bottom
        )
          dialog.close();
      }
    });
    dialog.addEventListener("close", () => {
      // Native close events can arrive after another showModal().
      if (dialog.open) return;
      cancel();
      input.removeAttribute("aria-busy");
      const target =
        trigger?.closest("[data-core-sidebar]") && innerWidth < 1024
          ? document.querySelector(
              "[data-core-mobile],#mobile-menu,[data-mobile]",
            )
          : trigger;
      if (target?.isConnected) target.focus({ preventScroll: true });
    });
    document.addEventListener("click", (e) => {
      const button = e.target.closest("[data-command]");
      if (button) {
        e.preventDefault();
        open(button);
      }
    });
    document.addEventListener("keydown", (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        open();
      }
    });
    document
      .querySelectorAll("[data-shortcut]")
      .forEach(
        (e) =>
          (e.textContent = /Mac|iPhone|iPad/.test(navigator.platform)
            ? "⌘ K"
            : "Ctrl K"),
      );
  }
  window.KagNodeSearch = { open };
  mount();
})();
