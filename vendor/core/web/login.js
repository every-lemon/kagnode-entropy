/*! KagNode Core — optional host-owned login connection presentation. */
(() => {
  if (window.KagNodeLogin) return;
  // Usually captured by the synchronous head bootstrap. This fallback also
  // covers applications that only embed the deferred UI entrypoint.
  let resume = window.__kagNodeLoginReturn || null;
  delete window.__kagNodeLoginReturn;
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
      resume = returned;
    }
  } catch {
    resume = null;
  }
  const completed = new WeakSet();
  const progressive = new WeakMap();
  let current = null,
    generation = 0,
    readyHost = null;
  const clear = () => {
    if (!current) return;
    clearTimeout(current.timer);
    clearTimeout(current.expiryTimer);
    current.button.disabled = false;
    current.button.removeAttribute("aria-busy");
    current = null;
  };
  function status(state, message) {
    state.status.replaceChildren(document.createTextNode(message));
  }
  function cancelAttempt(state) {
    if (!state?.attempt || state.cancelled || state.redirecting) return;
    state.cancelled = true;
    try {
      Promise.resolve(
        state.host.request("cancel", { attempt_id: state.attempt }),
      ).catch(() => {});
    } catch {}
  }
  function terminal(state, message) {
    if (current !== state) return;
    generation++;
    cancelAttempt(state);
    status(state, message);
    clear();
  }
  async function receive(state, result) {
    if (current !== state || state.generation !== generation) return;
    if (result?.authorized === true) {
      status(state, "Identity confirmed. Continuing…");
      try {
        await state.host.complete(result);
        if (current === state) {
          generation++;
          clear();
          completed.add(state.button);
          state.button.disabled = true;
        }
      } catch {
        terminal(state, "Unable to continue. Try again or sign in locally.");
      }
      return;
    }
    if (!result?.pending) {
      terminal(
        state,
        "Connection was not approved. Try again or sign in locally.",
      );
      return;
    }
    if (!state.attempt) {
      if (result.authorization_uri !== undefined) {
        if (validAttempt(result.attempt_id)) state.attempt = result.attempt_id;
        let link;
        try {
          link = new URL(result.authorization_uri);
          const issuer = new URL(
            state.host.issuer ||
              state.host.state?.()?.issuer ||
              "https://nebulacoder.com",
          );
          if (
            link.protocol !== "https:" ||
            issuer.protocol !== "https:" ||
            link.origin !== issuer.origin ||
            link.username ||
            link.password ||
            issuer.username ||
            issuer.password ||
            link.hash ||
            !validAttempt(result.attempt_id)
          )
            throw Error("Invalid issuer redirect");
        } catch {
          return terminal(
            state,
            "Unable to connect. Try again or sign in locally.",
          );
        }
        state.attempt = result.attempt_id;
        state.redirecting = true;
        status(state, "Opening KagNode…");
        try {
          location.assign(link.href);
        } catch {
          state.redirecting = false;
          terminal(
            state,
            "Unable to open KagNode. Try again or sign in locally.",
          );
        }
        return;
      }
      if (
        typeof result.attempt_id !== "string" ||
        !result.attempt_id ||
        typeof result.user_code !== "string"
      )
        return terminal(
          state,
          "Unable to connect. Try again or sign in locally.",
        );
      let link;
      try {
        link = new URL(result.verification_uri);
        if (link.protocol !== "https:" || link.username || link.password)
          throw Error("Invalid verification link");
      } catch {
        return terminal(
          state,
          "Unable to connect. Try again or sign in locally.",
        );
      }
      state.attempt = result.attempt_id;
      const lifetime = Number(result.expires_in);
      state.deadline =
        Date.now() +
        Math.min(600, Math.max(1, Number.isFinite(lifetime) ? lifetime : 300)) *
          1000;
      clearTimeout(state.expiryTimer);
      state.expiryTimer = setTimeout(
        () =>
          terminal(state, "Connection expired. Try again or sign in locally."),
        Math.max(1, state.deadline - Date.now()),
      );
      state.interval =
        Math.min(30, Math.max(1, Number(result.interval) || 3)) * 1000;
      status(state, "Confirm this code with KagNode: ");
      const code = document.createElement("strong");
      code.textContent = result.user_code;
      const verification = document.createElement("a");
      verification.href = link.href;
      verification.target = "_blank";
      verification.rel = "noopener noreferrer";
      verification.textContent = "Open KagNode to approve";
      const cancel = document.createElement("button");
      cancel.type = "button";
      cancel.textContent = "Cancel connection";
      cancel.addEventListener("click", () => {
        if (current !== state) return;
        terminal(
          state,
          "Connection cancelled. You can try again or sign in locally.",
        );
        state.button.focus({ preventScroll: true });
      });
      state.status.append(
        code,
        document.createTextNode(" "),
        verification,
        document.createTextNode(" "),
        cancel,
      );
    }
    if (!state.deadline) {
      state.deadline = Date.now() + 30000;
      state.interval = 1000;
    }
    state.timer = setTimeout(
      async () => {
        if (current !== state) return;
        if (Date.now() >= state.deadline)
          return terminal(
            state,
            "Connection expired. Try again or sign in locally.",
          );
        try {
          await receive(
            state,
            await state.host.request("status", { attempt_id: state.attempt }),
          );
        } catch {
          terminal(
            state,
            "Unable to check the connection. Try again or sign in locally.",
          );
        }
      },
      Math.min(state.interval, Math.max(1, state.deadline - Date.now())),
    );
  }
  document.addEventListener("click", async (event) => {
    const button = event.target.closest?.("button[data-core-login-provider]");
    const host = window.KagNodeLoginHost;
    if (!button || !host?.request || !host?.complete) return;
    const container = button.closest("[data-core-login]");
    const output = container?.querySelector("[data-core-login-status]");
    if (
      !output ||
      current ||
      completed.has(button) ||
      host.state?.()?.enabled === false ||
      host.state?.()?.ready === false
    )
      return;
    event.preventDefault();
    resume = null;
    const state = { button, status: output, host, generation: ++generation };
    current = state;
    state.expiryTimer = setTimeout(
      () => terminal(state, "Unable to connect. Try again or sign in locally."),
      30000,
    );
    button.disabled = true;
    button.setAttribute("aria-busy", "true");
    output.setAttribute("role", "status");
    status(state, "Connecting with KagNode…");
    try {
      await receive(state, await host.request("start", {}));
    } catch {
      terminal(state, "Unable to connect. Try again or sign in locally.");
    }
  });
  function validAttempt(value) {
    return typeof value === "string" && /^[A-Za-z0-9._~-]{1,512}$/.test(value);
  }
  function refreshButtons() {
    const host = window.KagNodeLoginHost;
    if (!host) return;
    for (const button of document.querySelectorAll(
      "button[data-core-login-provider]",
    ))
      if (button !== current?.button)
        button.disabled =
          completed.has(button) ||
          host.state?.()?.enabled === false ||
          host.state?.()?.ready === false;
  }
  async function resumeReturn() {
    const host = window.KagNodeLoginHost;
    if (
      !resume ||
      current ||
      !host ||
      host !== readyHost ||
      !host.request ||
      !host.complete ||
      host.state?.()?.ready === false
    )
      return;
    const button = document.querySelector("button[data-core-login-provider]");
    const output = button
      ?.closest("[data-core-login]")
      ?.querySelector("[data-core-login-status]");
    if (!button || !output) return;
    const returned = resume;
    resume = null;
    if (!validAttempt(returned.attempt) || host.state?.()?.enabled === false) {
      output.textContent =
        "Connection was not approved. Try again or sign in locally.";
      return;
    }
    const state = {
      button,
      status: output,
      host,
      resuming: true,
      attempt: returned.attempt,
      generation: ++generation,
      deadline: Date.now() + 30000,
      interval: 1000,
    };
    current = state;
    button.disabled = true;
    button.setAttribute("aria-busy", "true");
    status(state, "Checking your KagNode connection…");
    state.expiryTimer = setTimeout(
      () =>
        terminal(state, "Connection expired. Try again or sign in locally."),
      30000,
    );
    try {
      await receive(
        state,
        await host.request("status", { attempt_id: state.attempt }),
      );
    } catch {
      terminal(
        state,
        "Unable to check the connection. Try again or sign in locally.",
      );
    }
  }
  function refresh() {
    readyHost = window.KagNodeLoginHost || null;
    if (current && current.host !== readyHost)
      terminal(current, "Connection changed. Try again or sign in locally.");
    enhanceForms();
    refreshButtons();
    resumeReturn();
  }
  function reset(container = document) {
    const buttons = [
      ...(container.matches?.("button[data-core-login-provider]")
        ? [container]
        : []),
      ...(container.querySelectorAll?.("button[data-core-login-provider]") ||
        []),
    ];
    if (buttons.length) resume = null;
    if (current && buttons.includes(current.button)) {
      generation++;
      cancelAttempt(current);
      clear();
    }
    for (const button of buttons) {
      completed.delete(button);
      button
        .closest("[data-core-login]")
        ?.querySelector("[data-core-login-status]")
        ?.replaceChildren();
    }
    enhanceForms();
    refreshButtons();
  }
  function enhanceForms() {
    for (const form of document.querySelectorAll("[data-core-login] form")) {
      const explicit = form.hasAttribute("data-core-login-progressive");
      const secrets = [
        ...form.querySelectorAll('input[type="password"]:not(:disabled)'),
      ];
      const secret = secrets[0];
      const identity = form.querySelector(
        '[data-core-login-identity]:not(:disabled), input[autocomplete="username"]:not(:disabled), input[autocomplete="email"]:not(:disabled), input[type="email"]:not(:disabled)',
      );
      const credentials = form.querySelector("[data-core-login-credentials]");
      const otp = [
        ...form.querySelectorAll(
          '[autocomplete="one-time-code"]:not(:disabled)',
        ),
      ];
      if (
        form.dataset.coreLoginProgressive === "off" ||
        secrets.length !== 1 ||
        !identity ||
        form.querySelector('[autocomplete="new-password"]:not(:disabled)') ||
        (!explicit && secret.autocomplete !== "current-password") ||
        (credentials &&
          (!credentials.contains(secret) || credentials.contains(identity))) ||
        otp.some((input) => !credentials?.contains(input))
      ) {
        progressive.get(form)?.destroy();
        continue;
      }
      const existing = progressive.get(form);
      if (existing) {
        existing.sync();
        continue;
      }
      const submits = [
        ...form.querySelectorAll(
          'button:not([type]), button[type="submit"], input[type="submit"]',
        ),
      ];
      if (!submits.length || submits.some((button) => button.hidden)) continue;
      const group = (input) => {
        const known = input.closest(".kn-core-login-field");
        if (
          known &&
          known.closest("form") === form &&
          !known.contains(input === identity ? secret : identity)
        )
          return known;
        const wrapper = document.createElement("div");
        wrapper.className = "kn-core-login-field";
        const control = input.closest(".kn-core-login-secret") || input;
        const label = input.labels?.[0];
        const first = label && form.contains(label) ? label : control;
        first.before(wrapper);
        if (label && form.contains(label)) wrapper.append(label);
        if (!wrapper.contains(control)) wrapper.append(control);
        return wrapper;
      };
      const identityGroup = group(identity),
        secretGroup = credentials || group(secret);
      const next = document.createElement("button");
      next.type = "button";
      next.dataset.coreLoginSubmit = "";
      next.textContent = "Continue";
      const summary = document.createElement("div");
      summary.className = "kn-core-login-identity";
      const value = document.createElement("span"),
        change = document.createElement("button");
      change.type = "button";
      change.className = "kn-core-login-change";
      change.textContent = "Change";
      change.setAttribute("aria-label", "Change sign-in identity");
      summary.append(value, change);
      identityGroup.before(summary);
      identityGroup.after(next);
      const originalNoValidate = form.noValidate;
      const originalStage = form.getAttribute("data-core-login-stage");
      const controller = new AbortController();
      const on = (target, name, handler, capture = false) =>
        target.addEventListener(name, handler, {
          capture,
          signal: controller.signal,
        });
      let animation;
      const state = { stage: "identity" };
      progressive.set(form, state);
      const show = (stage, focus = true) => {
        state.stage = stage;
        form.dataset.coreLoginStage = stage;
        const password = stage === "password";
        identityGroup.hidden = password;
        summary.hidden = !password;
        secretGroup.hidden = !password;
        next.hidden = password;
        submits.forEach((button) => (button.hidden = !password));
        form.noValidate = password ? originalNoValidate : true;
        value.textContent = identity.value;
        change.hidden = identity.readOnly;
        animation?.cancel();
        const visible = password ? secretGroup : identityGroup;
        if (!matchMedia("(prefers-reduced-motion: reduce)").matches)
          animation = visible.animate?.(
            [
              { opacity: 0, transform: "translateY(4px)" },
              { opacity: 1, transform: "translateY(0)" },
            ],
            { duration: 150, easing: "ease-out" },
          );
        if (focus)
          (password ? secret : identity).focus({ preventScroll: true });
      };
      const advance = (focus = true) => {
        if (!originalNoValidate && !identity.reportValidity()) return false;
        show("password", focus);
        return true;
      };
      on(next, "click", () => advance());
      on(change, "click", () => show("identity"));
      on(identity, "keydown", (event) => {
        if (
          event.key === "Enter" &&
          state.stage === "identity" &&
          !event.isComposing
        ) {
          event.preventDefault();
          advance();
        }
      });
      on(
        form,
        "submit",
        (event) => {
          if (state.stage !== "identity") return;
          const filled = !!secret.value;
          if (originalNoValidate || identity.reportValidity()) show("password");
          if (
            !filled ||
            state.stage !== "password" ||
            (!originalNoValidate && !form.reportValidity())
          ) {
            event.preventDefault();
            event.stopImmediatePropagation();
          }
        },
        true,
      );
      on(
        form,
        "invalid",
        (event) => {
          if (event.target === identity && state.stage === "password")
            show("identity");
        },
        true,
      );
      const autofill = () => {
        if (state.stage === "identity" && secret.value) show("password", false);
        value.textContent = identity.value;
      };
      on(form, "input", autofill);
      on(form, "change", autofill);
      show(
        secret.value || form.dataset.coreLoginStage === "password"
          ? "password"
          : "identity",
        false,
      );
      state.show = show;
      state.advance = advance;
      state.sync = () => {
        value.textContent = identity.value;
        change.hidden = identity.readOnly;
        if (identity.readOnly && identity.value && state.stage === "identity")
          advance(false);
      };
      state.destroy = () => {
        controller.abort();
        animation?.cancel();
        identityGroup.hidden = secretGroup.hidden = false;
        submits.forEach((button) => (button.hidden = false));
        summary.remove();
        next.remove();
        form.noValidate = originalNoValidate;
        if (originalStage === null)
          form.removeAttribute("data-core-login-stage");
        else form.setAttribute("data-core-login-stage", originalStage);
        progressive.delete(form);
      };
      state.sync();
    }
  }
  window.KagNodeLogin = {
    refresh,
    reset,
    advance(form, { focus = true } = {}) {
      enhanceForms();
      return progressive.get(form)?.advance(focus) || false;
    },
    get hasPendingReturn() {
      return !!resume || !!current?.resuming;
    },
  };
  document.addEventListener("kagnode:login-ready", refresh);
  enhanceForms();
  refreshButtons();
  if (window.KagNodeLoginHost?.state?.()?.ready === true) refresh();
  addEventListener("pagehide", () => {
    resume = null;
    readyHost = null;
    generation++;
    cancelAttempt(current);
    clear();
  });
})();
