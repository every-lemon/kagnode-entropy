/*! KagNode Core — AGPL-3.0-only. Appearance never contains vault data. */
(() => {
  // Runs synchronously in the head, before the deferred UI and its links.
  // The opaque local attempt is kept only in this document, never storage.
  try {
    const url = new URL(location.href);
    if (
      url.searchParams.has("kagnode_attempt") ||
      url.searchParams.has("kagnode_error")
    ) {
      const returned = {
        attempt:
          url.searchParams.getAll("kagnode_attempt").length === 1
            ? url.searchParams.get("kagnode_attempt")
            : null,
        failed: url.searchParams.has("kagnode_error"),
      };
      url.searchParams.delete("kagnode_attempt");
      url.searchParams.delete("kagnode_error");
      history.replaceState(history.state, "", url);
      window.__kagNodeLoginReturn = returned;
    }
  } catch {}
  if (window.KagNodeTheme) return;
  const root = document.documentElement;
  root.classList.add("js");
  root.dataset.knCore = "1.0.0";
  const system = matchMedia("(prefers-color-scheme: dark)");
  let session = { authenticated: false, subject: "" },
    preference = "system";
  const read = (key) => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  };
  const valid = (v) => ["system", "light", "dark"].includes(v);
  const navigationChoice = read("kagnode.core.sidebar.choice");
  root.dataset.coreCollapsed = String(
    navigationChoice === "collapsed" ||
      (!navigationChoice &&
        innerWidth >= 1024 &&
        innerWidth < 1360 &&
        matchMedia("(hover: hover) and (pointer: fine)").matches),
  );
  function apply(value = preference) {
    preference = valid(value) ? value : "system";
    const theme =
      !session.authenticated || preference === "system"
        ? system.matches
          ? "dark"
          : "light"
        : preference;
    root.dataset.theme = theme;
    root.style.colorScheme = theme;
    root.dataset.coreThemePreference = session.authenticated
      ? preference
      : "system";
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.content = theme === "dark" ? "#101010" : "#ffffff";
    document.dispatchEvent(
      new CustomEvent("kagnode:theme", { detail: { theme, preference } }),
    );
  }
  window.KagNodeTheme = {
    apply,
    get session() {
      return session;
    },
    setSession(value) {
      session = {
        authenticated: !!value.authenticated,
        subject: String(value.subject || ""),
      };
      const saved = read("kagnode.core.theme." + session.subject);
      apply(
        session.authenticated
          ? value.theme || root.dataset.accountTheme || saved || "system"
          : "system",
      );
    },
    saveLocal(value) {
      if (!session.authenticated || !valid(value)) return false;
      try {
        localStorage.setItem("kagnode.core.theme." + session.subject, value);
      } catch {}
      apply(value);
      return true;
    },
  };
  system.addEventListener("change", () => apply());
  apply("system");
})();
