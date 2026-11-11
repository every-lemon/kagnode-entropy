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
      '<svg class="kn-core-spinner-orbit" viewBox="0 0 96 96" fill="none" focusable="false" aria-hidden="true">' +
      '<circle class="kn-core-spinner-track" cx="48" cy="48" r="39"/>' +
      '<g class="kn-core-spinner-rotor"><path class="kn-core-spinner-sweep" d="M14.23 67.5A39 39 0 0 1 67.5 14.23"/>' +
      '<path class="kn-core-spinner-node" d="m83 39 6 7-3 10-6-6Z"/></g></svg>';
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
  window.KagNodeLoading = { spinner, enhance };
})();
