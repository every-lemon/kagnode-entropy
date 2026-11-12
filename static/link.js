"use strict";
let csrf = "";
async function api(path, body) {
  const response = await fetch("/api/v1/admin/" + path, {
    method: body ? "POST" : "GET",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const value = await response.json();
  if (!response.ok)
    throw Object.assign(
      new Error(
        value.error?.message ||
          "Sign in locally before linking, and confirm your administrator key.",
      ),
      { code: value.error?.code },
    );
  return value;
}
window.KagNodeLoginHost = {
  state: () => ({
    ready: true,
    enabled: true,
    issuer: "https://nebulacoder.com",
  }),
  async request(action, payload) {
    if (action === "start") {
      const session = await api("status");
      csrf = session.csrf;
      const credential = document.querySelector("#credential").value;
      document.querySelector("#credential").value = "";
      if (!credential) throw new Error("Administrator key required");
      return api("kagnode/start", { credential });
    }
    try {
      return await api(
        "kagnode/" + (action === "status" ? "poll" : "cancel"),
        payload,
      );
    } catch (error) {
      if (action === "status" && error.code === "authorization_denied")
        return { pending: false, authorized: false };
      throw error;
    }
  },
  async complete() {
    location.assign("/admin");
  },
};
document
  .querySelector("#admin-login-form")
  .addEventListener("submit", (event) => {
    event.preventDefault();
    document.querySelector("[data-core-login-provider]").click();
  });

document.dispatchEvent(new Event("kagnode:login-ready"));
