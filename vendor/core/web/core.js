/*! KagNode Core — shared shell and component lifecycle, AGPL-3.0-only. */
(() => {
  if (window.KagNodeCore) return;
  const root = document.documentElement,
    reduce = matchMedia("(prefers-reduced-motion: reduce)");
  const config = JSON.parse(
    document.querySelector('meta[name="kagnode-core"]')?.content || "{}",
  );
  root.dataset.knCore = config.version || "1.0.0";
  const $ = (q, parent = document) => parent.querySelector(q);
  const all = (q, parent = document) => [...parent.querySelectorAll(q)];
  const saved = (key) => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  };
  const store = (key, value) => {
    try {
      localStorage.setItem(key, value);
    } catch {}
  };
  const mobile = () => innerWidth < 1024;
  const hover = matchMedia("(hover: hover) and (pointer: fine)");
  const compactDefault = () => !mobile() && innerWidth < 1360 && hover.matches;
  const panelIcon =
    '<svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2"/><path class="kn-core-panel-divider" d="M8 4v16"/></svg>';
  const menuIcon =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true">' +
    window.KagNodeIcons.menu +
    "</svg>";
  const userIcon =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true">' +
    window.KagNodeIcons.account +
    "</svg>";
  let sidebar,
    toggle,
    scrim,
    popup = null,
    accountTrigger = null,
    lastFocus;
  const groups = new WeakMap();
  let manualChoice = saved("kagnode.core.sidebar.choice"),
    desktopCollapsed = manualChoice
      ? manualChoice === "collapsed"
      : compactDefault(),
    previewExpanded = false,
    hoverTimer;
  function preview(open) {
    clearTimeout(hoverTimer);
    if (
      !sidebar ||
      mobile() ||
      !desktopCollapsed ||
      document.querySelector("dialog[open]")
    )
      open = false;
    previewExpanded = !!open;
    root.dataset.corePreview = String(previewExpanded);
    root.dataset.coreCollapsed = String(
      !mobile() && desktopCollapsed && !previewExpanded,
    );
    root.classList.toggle(
      "collapsed",
      !mobile() && desktopCollapsed && !previewExpanded,
    );
    if (toggle) {
      toggle.setAttribute(
        "aria-expanded",
        String(!desktopCollapsed || previewExpanded),
      );
      toggle.setAttribute(
        "aria-label",
        previewExpanded
          ? "Keep sidebar open"
          : desktopCollapsed
            ? "Expand sidebar"
            : "Collapse sidebar",
      );
    }
    window.KagNodeScroll?.refresh(sidebar);
  }
  function releasePreview() {
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      if (
        !sidebar?.matches(":hover") &&
        !sidebar?.contains(document.activeElement)
      )
        preview(false);
    }, 180);
  }
  function element(tag, cls, text) {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text) el.textContent = text;
    return el;
  }
  function publicSidebar() {
    const aside = element("aside", "sidebar");
    aside.id = "sidebar";
    const brand = element("a", "brand");
    brand.href = "/";
    const image = element("img");
    image.src = config.mark || (config.base || "") + "mark.svg";
    image.alt = "";
    brand.append(image, element("strong", "", "KagNode."));
    aside.append(brand);
    const nav = element("nav");
    nav.setAttribute("aria-label", "KagNode pages");
    const supplied = all(".site-navigation .nav-pages > a");
    const links = [
      { href: "/", text: "Home" },
      ...supplied.map((a) => ({
        href: a.getAttribute("href"),
        text:
          $(".nav-page-label", a)?.textContent.trim() || a.textContent.trim(),
      })),
      { href: "/open-source", text: "Open source" },
      { href: "/demos", text: "Product examples" },
      { href: "/support", text: "Support" },
      {
        href: config.authenticated ? "/account" : "/login",
        text: config.authenticated ? "Your account" : "Sign in",
      },
    ];
    const seen = new Set();
    for (const item of links) {
      if (!item.href || seen.has(item.href)) continue;
      seen.add(item.href);
      const a = element("a", "", item.text);
      a.href = item.href;
      if (location.pathname === item.href)
        a.setAttribute("aria-current", "page");
      nav.append(a);
    }
    aside.append(nav);
    document.body.prepend(aside);
    root.dataset.corePublic = "true";
    return aside;
  }
  function shellActive() {
    if (!sidebar) return;
    const visible =
      !sidebar.closest("[hidden]") &&
      getComputedStyle(sidebar).display !== "none";
    const state = visible ? "active" : "inactive";
    if (root.dataset.coreShell !== state) {
      root.dataset.coreShell = state;
      requestAnimationFrame(() => window.KagNodeScroll?.refresh());
    }
  }
  function setCollapsed(value, persist = true) {
    clearTimeout(hoverTimer);
    previewExpanded = false;
    root.dataset.corePreview = "false";
    desktopCollapsed = !!value;
    if (persist) {
      manualChoice = desktopCollapsed ? "collapsed" : "expanded";
      store("kagnode.core.sidebar.choice", manualChoice);
    }
    value = !mobile() && desktopCollapsed;
    root.dataset.coreCollapsed = String(!!value);
    root.classList.toggle("collapsed", !!value);
    if (toggle) {
      toggle.setAttribute("aria-expanded", String(!value));
      toggle.setAttribute(
        "aria-label",
        value ? "Expand sidebar" : "Collapse sidebar",
      );
      toggle.title = value ? "Expand sidebar" : "Collapse sidebar";
    }

    if (value)
      for (const d of all("details", sidebar)) {
        groups.get(d)?.motion?.cancel();
        groups.delete(d);
        d.open = false;
        $("summary", d)?.setAttribute("aria-expanded", "false");
        if (d.children[1]) {
          d.children[1].style.removeProperty("height");
          d.children[1].style.removeProperty("overflow");
          d.children[1].inert = true;
        }
      }
    document.dispatchEvent(
      new CustomEvent("kagnode:sidebar", { detail: { collapsed: !!value } }),
    );
    window.KagNodeScroll?.refresh(sidebar);
  }
  function drawer(open, restore = true) {
    root.dataset.coreDrawer = String(open);
    scrim.hidden = !open;
    sidebar.inert = mobile() && !open;
    all("[data-mobile],#mobile-menu,[data-core-mobile]").forEach((b) =>
      b.setAttribute("aria-expanded", String(open)),
    );
    if (open) {
      lastFocus = document.activeElement;
      $("a,button,summary", sidebar)?.focus();
    } else if (restore) lastFocus?.focus?.({ preventScroll: true });
    requestAnimationFrame(() => window.KagNodeScroll?.refresh());
  }
  function installSidebar() {
    sidebar = $("#sidebar,.sidebar");
    if (!sidebar && config.publicNavigation) sidebar = publicSidebar();
    if (!sidebar || sidebar.dataset.coreSidebar) return;
    sidebar.dataset.coreSidebar = "true";
    sidebar.setAttribute(
      "aria-label",
      sidebar.getAttribute("aria-label") || "Workspace navigation",
    );
    let brand = $(".brand,.sidebar-brand,.workspace-brand", sidebar);
    if (!brand) {
      brand = element("a", "brand");
      brand.href = config.home || "https://nebulacoder.com";
      const image = element("img");
      image.src = config.mark || (config.base || "") + "mark.svg";
      image.alt = "";
      brand.append(image, element("strong", "", "KagNode."));
    }
    const brandContainer = [...sidebar.children].find(
      (child) => child === brand || child.contains(brand),
    );
    const heading = element("div", "kn-core-sidebar-heading");
    heading.append(brand);
    const close = brandContainer?.querySelector(
      "[data-dismiss-nav],#sidebar-close",
    );
    if (close) heading.append(close);
    const nav = element("div", "kn-core-sidebar-navigation");
    nav.setAttribute("aria-label", "Workspace pages");
    const legacy = element("div");
    legacy.hidden = true;
    legacy.dataset.coreLegacyMetadata = "";
    for (const child of [...sidebar.children]) {
      if (child === brand || child === brandContainer || child.contains(brand))
        continue;
      if (
        child.matches(".sidebar-footer,.sidebar-bottom,.kn-product-signature")
      ) {
        legacy.append(child);
        continue;
      }
      nav.append(child);
    }
    for (const d of all("details", nav)) {
      d.dataset.coreGroup = d.dataset.group || "";
      d.removeAttribute("data-group");
      d.classList.remove("nav-group");
      d.classList.add("kn-core-nav-group");
      const summary = $("summary", d);
      if (summary) summary.setAttribute("aria-expanded", String(d.open));
    }
    const footer = element("div", "kn-core-sidebar-footer");
    toggle = element("button", "kn-core-sidebar-toggle");
    toggle.type = "button";
    toggle.dataset.coreToggle = "";
    toggle.innerHTML = panelIcon;
    footer.append(toggle);
    sidebar.replaceChildren(heading, nav, footer, legacy);
    scrim = element("button", "kn-core-drawer-scrim");
    scrim.type = "button";
    scrim.hidden = true;
    scrim.setAttribute("aria-label", "Close navigation");
    document.body.append(scrim);
    scrim.addEventListener("click", () => drawer(false));
    setCollapsed(desktopCollapsed, false);
    sidebar.addEventListener("pointerenter", () => {
      clearTimeout(hoverTimer);
      if (hover.matches && !mobile() && desktopCollapsed)
        hoverTimer = setTimeout(() => preview(true), 120);
    });
    sidebar.addEventListener("pointerleave", releasePreview);
    sidebar.addEventListener("focusin", (e) => {
      if (
        !e.target.closest("[data-core-toggle]") &&
        desktopCollapsed &&
        !mobile()
      )
        preview(true);
    });
    sidebar.addEventListener("focusout", releasePreview);
    let mobileButton = $("[data-mobile],#mobile-menu");
    if (!mobileButton) {
      mobileButton = element("button", "kn-core-mobile-toggle");
      mobileButton.type = "button";
      mobileButton.dataset.coreMobile = "";
      mobileButton.innerHTML = menuIcon;
      mobileButton.setAttribute("aria-label", "Open navigation");
      mobileButton.setAttribute("aria-controls", sidebar.id);
      document.body.append(mobileButton);
    }
    sidebar.inert = mobile();
    shellActive();
  }
  function groupToggle(d) {
    const summary = $("summary", d),
      content = d.children[1];
    if (!content) return;
    const old = groups.get(d);
    const from = content.getBoundingClientRect().height;
    const open = !(old?.open ?? d.open);
    old?.motion?.cancel();
    d.open = true;
    content.inert = !open;
    summary?.setAttribute("aria-expanded", String(open));
    const to = open ? content.scrollHeight : 0;
    const finish = () => {
      d.open = open;
      content.style.removeProperty("height");
      content.style.removeProperty("overflow");
      groups.delete(d);
      window.KagNodeScroll?.refresh(d);
    };
    if (reduce.matches) return finish();
    content.style.overflow = "hidden";
    const motion = content.animate(
      [
        { height: from + "px", opacity: open ? 0.5 : 1 },
        { height: to + "px", opacity: open ? 1 : 0 },
      ],
      { duration: 200, easing: "cubic-bezier(.2,.8,.2,1)" },
    );
    groups.set(d, { open, motion });
    motion.finished
      .then(() => {
        if (groups.get(d)?.motion === motion) finish();
      })
      .catch(() => {});
  }
  function closeAccount(focus = false) {
    if (!popup) return;
    popup.hidden = true;
    popup.classList.add("hidden");
    popup.inert = true;
    accountTrigger?.setAttribute("aria-expanded", "false");
    if (focus) accountTrigger?.focus();
    popup = null;
  }
  function openAccount(button) {
    if (popup && accountTrigger === button) return closeAccount(true);
    closeAccount();
    accountTrigger = button;
    let menu = $(
      "#account-menu,[data-core-menu-content],#kn-core-account-menu",
    );
    if (!menu) {
      menu = element("section", "kn-core-account-menu");
      menu.id = "kn-core-account-menu";
      const identity = element("div", "kn-core-account-identity");
      identity.append(
        element(
          "strong",
          "",
          config.name ||
            $("[data-kn-account-name]")?.textContent.trim() ||
            "Your account",
        ),
      );
      const email =
        config.email ||
        (window.KagNodeTheme?.session.subject.includes("@")
          ? window.KagNodeTheme.session.subject
          : "");
      if (email) identity.append(element("small", "", email));
      menu.append(identity);
      const settings = element("a", "", "Account & preferences");
      settings.href =
        document.body.dataset.coreSettings ||
        (document.body.dataset.knAccountApp === "mail"
          ? "#account"
          : document.body.dataset.knAccountApp === "httpboot"
            ? "/access"
            : document.body.dataset.knAccountApp === "vpn"
              ? "/manage?view=system"
              : "/settings");
      settings.dataset.view = "settings";
      menu.append(settings);
      const lock = $("#lock-button,#logout-button,[data-signout]");
      if (lock) {
        const action = element(
          "button",
          "",
          lock.textContent.trim() || "Sign out",
        );
        action.type = "button";
        action.addEventListener("click", () => {
          closeAccount();
          lock.click();
        });
        menu.append(action);
      }
      document.body.append(menu);
    }
    if (menu.id === "kn-core-account-menu") {
      const name =
        $("[data-kn-account-name]")?.textContent.trim() ||
        config.name ||
        "Your account";
      const identity = $(".kn-core-account-identity", menu);
      $("strong", identity).textContent = name;
      let email = $("small", identity);
      if (config.email && !email) {
        email = element("small");
        identity.append(email);
      }
      if (email) {
        email.textContent = config.email || "";
        email.hidden = !config.email;
      }
    }
    if (!menu.id) menu.id = "kn-core-account-menu-content";
    menu.dataset.coreAccountMenu = "";
    menu.hidden = false;
    menu.classList.remove("hidden");
    menu.inert = false;
    menu.setAttribute("aria-label", "Account");
    document.body.append(menu);
    popup = menu;
    button.setAttribute("aria-controls", menu.id);
    button.setAttribute("aria-expanded", "true");
    const box = button.getBoundingClientRect();
    const maximumRight = Math.max(12, innerWidth - menu.offsetWidth - 12);
    Object.assign(menu.style, {
      top: Math.min(innerHeight - 100, box.bottom + 8) + "px",
      right:
        Math.min(maximumRight, Math.max(12, innerWidth - box.right)) + "px",
    });
    if (!reduce.matches)
      menu.animate(
        [
          { opacity: 0, transform: "translateY(-4px) scale(.98)" },
          { opacity: 1, transform: "none" },
        ],
        { duration: 160, easing: "ease-out" },
      );
    $("a,button", menu)?.focus({ preventScroll: true });
  }
  function componentIcon(name) {
    const key = Object.hasOwn(window.KagNodeIcons, name) ? name : "plan";
    return (
      '<svg class="ui-icon" data-core-icon-name="' +
      key +
      '" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' +
      window.KagNodeIcons[key] +
      "</svg>"
    );
  }
  function normalize() {
    for (const host of all("[data-core-icon]")) {
      const name = host.dataset.coreIcon;
      const key = Object.hasOwn(window.KagNodeIcons, name) ? name : "plan";
      if (
        host.dataset.coreIconRendered === key &&
        host.childElementCount === 1 &&
        host.firstElementChild?.getAttribute("data-core-icon-name") === key
      )
        continue;
      host.innerHTML = componentIcon(key);
      host.dataset.coreIconRendered = key;
    }

    window.KagNodeLoading?.enhance();
    if (!sidebar || !sidebar.isConnected) {
      scrim?.remove();
      document.querySelector(".kn-core-mobile-toggle")?.remove();
      installSidebar();
    }
    if (sidebar) {
      const closed = mobile() && root.dataset.coreDrawer !== "true";
      if (sidebar.inert !== closed) sidebar.inert = closed;
    }
    if (
      root.dataset.coreDrawer === "true" &&
      document.querySelector("dialog[open]")
    )
      drawer(false, false);
    if (document.querySelector("dialog[open]")) {
      if (previewExpanded) preview(false);
      closeAccount();
    }
    if (sidebar?.closest("[hidden]") && root.dataset.coreDrawer === "true")
      drawer(false);
    all(
      "[data-theme-toggle],.theme-toggle,button[data-theme],#theme,#theme-toggle,[data-kn-theme]",
    ).forEach((b) => b.remove());
    for (const details of all("details.account-menu")) {
      const summary = $("summary", details),
        content = details.querySelector(":scope > div");
      if (!summary || !content || details.dataset.coreNativeMenu) continue;
      details.dataset.coreNativeMenu = "true";
      details.open = true;
      summary.dataset.coreAccountTrigger = "";
      content.dataset.coreMenuContent = "";
      content.id = "kn-core-native-account";
      content.hidden = true;
      const identity = element("div", "kn-core-account-identity");
      identity.append(
        element(
          "strong",
          "",
          $("[data-kn-account-name]", summary)?.textContent.trim() ||
            "Your account",
        ),
      );
      content.prepend(identity);
    }
    if (
      window.KagNodeTheme?.session.authenticated &&
      !$("[data-core-account-trigger]")
    ) {
      const header = $(".topbar-right,.account-controls,.topbar");
      if (header) {
        const avatar = $("[data-kn-account-avatar],#sidebar-avatar");
        const button = element("button", "icon-button");
        button.type = "button";
        button.dataset.coreAccountTrigger = "";
        if (avatar) {
          const parent = avatar.parentElement;
          if (parent?.closest(".topbar") && parent !== header) {
            parent.hidden = true;
            parent.before(button);
          } else header.append(button);
          button.append(avatar);
        } else {
          button.innerHTML = userIcon;
          header.append(button);
        }
      }
    }
    all(
      "[data-account-menu],button[data-kn-account-avatar],[data-core-account-trigger]",
    ).forEach((b) => {
      b.dataset.coreAccountTrigger = "";
      b.setAttribute("aria-label", "Open account menu");
      b.setAttribute("aria-haspopup", "true");
    });
    if (sidebar)
      for (const link of all(
        ".kn-core-sidebar-navigation a,.kn-core-sidebar-navigation summary",
        sidebar,
      )) {
        if (link.dataset.coreIconReady) continue;
        const label =
          link.getAttribute("aria-label") || link.textContent.trim();
        const explicit = $("svg[data-core-icon-name]", link)?.dataset
          .coreIconName;
        const choices = [
          [/inbox/i, "inbox"],
          [/sent|send|announcement/i, "send"],
          [/trash|removed/i, "trash"],
          [/archive/i, "archive"],
          [/draft|proposal|project/i, "projects"],
          [/meeting|recent/i, "clock"],
          [/people|member|user|team|discussion|communit/i, "community"],
          [/security|password|authenticat/i, "security"],
          [/setting|preference|account|manage/i, "settings"],
          [/source|developer|cli/i, "code"],
          [/overview|home|website|discover/i, "home"],
          [/image|photo|media/i, "images"],
          [/storage|database/i, "storage"],
          [/network|vpn/i, "networking"],
        ];
        const name =
            explicit ||
            choices.find(([test]) => test.test(label))?.[1] ||
            "plan",
          paths = window.KagNodeIcons[name] || window.KagNodeIcons.plan;
        const svg = document.createElementNS(
          "http://www.w3.org/2000/svg",
          "svg",
        );
        svg.setAttribute("viewBox", "0 0 24 24");
        svg.setAttribute("fill", "none");
        svg.setAttribute("stroke", "currentColor");
        svg.setAttribute("stroke-width", "1.7");
        svg.setAttribute("aria-hidden", "true");
        svg.innerHTML = paths;
        const previous = $(
          ":scope > svg,:scope > .ui-icon,:scope > .icon,:scope > [data-icon],:scope > [data-core-icon],:scope > .nav-icon",
          link,
        );
        if (previous?.matches("[data-icon],[data-core-icon]")) {
          // Dynamic applications redraw these hosts; keep one leading icon.
          previous.dataset.coreIconHost = "true";
          previous.replaceChildren(svg);
        } else if (previous) previous.replaceWith(svg);
        else link.prepend(svg);
        link.dataset.coreIconReady = "true";
        link.dataset.coreTooltip = label;
        link.removeAttribute("data-tooltip");
        link.setAttribute("aria-label", label);
      }
    if (
      window.KagNodeTheme?.session.authenticated &&
      $("[data-core-account-trigger]")
    ) {
      for (const button of all(".topbar #lock-button,.topbar #logout-button"))
        button.hidden = true;
    }
    document.body.removeAttribute("data-kn-manual-navigation");
    document.body.classList.remove("kn-navigation-collapsed");
    shellActive();
  }
  let tooltip;
  function hideTooltip() {
    if (tooltip) tooltip.hidden = true;
  }
  function showTooltip(target) {
    if (root.dataset.coreCollapsed !== "true" || mobile()) return;
    const label = target?.closest?.("[data-core-tooltip]");
    if (!label) return;
    if (!tooltip) {
      tooltip = element("div", "kn-core-tooltip");
      tooltip.role = "tooltip";
      tooltip.id = "kn-core-tooltip";
      document.body.append(tooltip);
    }
    tooltip.textContent = label.dataset.coreTooltip;
    tooltip.hidden = false;
    const r = label.getBoundingClientRect();
    Object.assign(tooltip.style, {
      left: r.right + 10 + "px",
      top: Math.max(8, Math.min(innerHeight - 44, r.top)) + "px",
    });
  }
  document.addEventListener("pointerover", (e) => showTooltip(e.target));
  document.addEventListener("focusin", (e) => showTooltip(e.target));
  document.addEventListener("pointerout", hideTooltip);
  document.addEventListener("focusout", hideTooltip);
  document.addEventListener("kagnode:sidebar", hideTooltip);
  document.addEventListener(
    "click",
    (e) => {
      const target = e.target instanceof Element ? e.target : null;
      if (!target) return;
      const button = target.closest(
        "[data-core-toggle],[data-mobile],#mobile-menu,[data-core-mobile],[data-dismiss-nav],#sidebar-close,.kn-navigation-toggle",
      );
      if (button && sidebar) {
        e.preventDefault();
        e.stopImmediatePropagation();
        if (button.matches("[data-dismiss-nav],#sidebar-close")) drawer(false);
        else if (mobile()) drawer(root.dataset.coreDrawer !== "true");
        else setCollapsed(!desktopCollapsed);
        return;
      }
      const summary = target.closest(".kn-core-nav-group > summary");
      if (summary) {
        e.preventDefault();
        e.stopImmediatePropagation();
        if (root.dataset.coreCollapsed === "true" && !mobile())
          setCollapsed(false);
        groupToggle(summary.parentElement);
        return;
      }
      const account = target.closest("[data-core-account-trigger]");
      if (account) {
        e.preventDefault();
        e.stopImmediatePropagation();
        openAccount(account);
        return;
      }
      if (popup && !popup.contains(target)) closeAccount();
      if (mobile() && sidebar?.contains(target) && target.closest("a[href]"))
        drawer(false);
    },
    true,
  );
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      if (popup) {
        e.preventDefault();
        closeAccount(true);
      }
      if (root.dataset.coreDrawer === "true") {
        e.preventDefault();
        drawer(false);
      }
    }
    if (popup && ["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
      const items = all("a[href],button:not(:disabled)", popup).filter(
        (item) =>
          !item.closest("[hidden],[inert]") && item.getClientRects().length,
      );
      e.preventDefault();
      const i = items.indexOf(document.activeElement);
      items[
        e.key === "Home"
          ? 0
          : e.key === "End"
            ? items.length - 1
            : (i + (e.key === "ArrowDown" ? 1 : -1) + items.length) %
              items.length
      ]?.focus();
    }
    if (e.key === "Tab" && mobile() && root.dataset.coreDrawer === "true") {
      const items = all(
        "a[href],button:not(:disabled),summary,input",
        sidebar,
      ).filter((n) => n.getClientRects().length);
      if (e.shiftKey && document.activeElement === items[0]) {
        e.preventDefault();
        items.at(-1)?.focus();
      } else if (!e.shiftKey && document.activeElement === items.at(-1)) {
        e.preventDefault();
        items[0]?.focus();
      }
    }
  });
  document.addEventListener("focusin", (e) => {
    if (popup && !popup.contains(e.target) && e.target !== accountTrigger)
      closeAccount();
  });
  // Pointer events inside embedded documents do not reach this document.
  // Dismiss without taking focus back from the frame or another window.
  addEventListener("blur", () => closeAccount());
  let layoutWidth = innerWidth;
  addEventListener("resize", () => {
    if (innerWidth === layoutWidth) {
      window.KagNodeScroll?.refresh();
      return;
    }
    layoutWidth = innerWidth;
    delete root.dataset.coreReady;
    closeAccount();
    if (sidebar) {
      setCollapsed(manualChoice ? desktopCollapsed : compactDefault(), false);
      sidebar.inert = mobile() && root.dataset.coreDrawer !== "true";
    }
    if (!mobile() && scrim) {
      root.dataset.coreDrawer = "false";
      scrim.hidden = true;
    }
    void document.body.offsetWidth;
    requestAnimationFrame(() => (root.dataset.coreReady = "true"));
    window.KagNodeScroll?.refresh();
  });
  hover.addEventListener("change", () => {
    if (!manualChoice) setCollapsed(compactDefault(), false);
  });
  let queued = false;
  new MutationObserver((records) => {
    if (
      records.every(
        (r) =>
          r.target.closest?.(".kn-core-rail") ||
          (r.type === "childList" &&
            [...r.addedNodes, ...r.removedNodes].every(
              (n) => n.nodeType !== 1,
            )),
      )
    )
      return;
    if (!queued) {
      queued = true;
      requestAnimationFrame(() => {
        queued = false;
        normalize();
      });
    }
  }).observe(document.body, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["hidden", "open", "inert", "data-core-icon"],
  });
  const loading = {
    show(label = "Loading…") {
      let el = $("#kn-core-loading");
      if (!el) {
        el = element("div", "kn-core-loading");
        el.id = "kn-core-loading";
        el.role = "status";
        el.setAttribute("aria-live", "polite");
        el.append(
          window.KagNodeLoading.spinner(),
          element("span", "kn-core-loading-label"),
        );
        document.body.append(el);
      }
      $(".kn-core-loading-label", el).textContent = label;
      el.hidden = false;
      root.classList.add("kag-loading-covered");
      document.dispatchEvent(new Event("kagnode:loading-start"));
    },
    hide() {
      const el = $("#kn-core-loading");
      if (el) el.hidden = true;
      root.classList.remove("kag-loading-covered");
      document.dispatchEvent(new Event("kagnode:loading-end"));
    },
  };
  window.KagNodeCore = {
    version: config.version || "1.0.0",
    config,
    icons: window.KagNodeIcons,
    iconSources: window.KagNodeIconSources,
    icon: componentIcon,
    loading,
    closeAccount,
    navigation: {
      open: () => drawer(true),
      close: () => drawer(false),
      collapse: setCollapsed,
    },
    refresh: normalize,
    setSession(value) {
      const old = window.KagNodeTheme?.session;
      const sameSubject = old?.subject === String(value.subject || "");
      if (!value.authenticated && root.dataset.coreDrawer === "true")
        drawer(false);
      config.name = value.authenticated
        ? String(value.name || (sameSubject ? config.name : "") || "")
        : "";
      config.email = value.authenticated
        ? String(value.email || (sameSubject ? config.email : "") || "")
        : "";
      if (
        old?.authenticated !== !!value.authenticated ||
        old?.subject !== String(value.subject || "")
      ) {
        closeAccount();
        $("#kn-core-account-menu")?.remove();
        window.KagNodeTheme?.setSession(value);
        document.dispatchEvent(
          new CustomEvent("kagnode:core-session", { detail: value }),
        );
      }
      normalize();
    },
    scrollTo(el) {
      el.scrollIntoView({
        behavior: reduce.matches ? "instant" : "smooth",
        block: "start",
      });
    },
  };
  installSidebar();
  normalize();
  window.KagNodeTheme?.setSession({
    authenticated: !!config.authenticated,
    subject: config.subject || "",
    theme: root.dataset.accountTheme || undefined,
  });
  normalize();
  void document.body.offsetWidth;
  requestAnimationFrame(() => (root.dataset.coreReady = "true"));
  document.dispatchEvent(new Event("kagnode:core-ready"));
})();
