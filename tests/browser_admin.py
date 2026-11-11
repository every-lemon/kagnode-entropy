"""Disposable administrator/browser QA; no external service or real user data."""

import json, os, signal, socket, subprocess, tempfile, time
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(
    os.environ.get("ENTROPY_BINARY", str(ROOT / "target/debug/kagnode-entropy"))
)
with tempfile.TemporaryDirectory(prefix="entropy-admin-browser-") as temporary:
    directory = Path(temporary) / "service"
    subprocess.run(
        [str(BINARY), "init", "--directory", str(directory)],
        check=True,
        capture_output=True,
    )
    credential = (directory / "admin.key").read_text()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    process = subprocess.Popen(
        [
            str(BINARY),
            "serve",
            "--port",
            str(port),
            "--credentials-file",
            str(directory / "credentials.json"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        import urllib.request

        for _ in range(80):
            try:
                urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/health", timeout=1
                ).close()
                break
            except OSError:
                time.sleep(0.1)
        with sync_playwright() as p:
            kind = os.environ.get(
                "KAGNODE_BROWSER_ENGINE", os.environ.get("BROWSER", "chromium")
            )
            options = {"headless": True}
            if kind == "chromium" and os.environ.get("PLAYWRIGHT_CHROMIUM_EXECUTABLE"):
                options["executable_path"] = os.environ[
                    "PLAYWRIGHT_CHROMIUM_EXECUTABLE"
                ]
            if os.environ.get("KAGNODE_BROWSER_EXECUTABLE"):
                options["executable_path"] = os.environ["KAGNODE_BROWSER_EXECUTABLE"]
            browser = getattr(p, kind).launch(**options)
            results = []
            artifacts = ROOT / ".runtime" / "browser-admin" / kind
            artifacts.mkdir(parents=True, exist_ok=True)
            for width in [390, 820, 1440]:
                context = browser.new_context(viewport={"width": width, "height": 900})
                page = context.new_page()
                errors = []
                external = []
                page.on("pageerror", lambda error: errors.append(str(error)))
                page.on(
                    "request",
                    lambda request: external.append(request.url)
                    if not request.url.startswith(f"http://127.0.0.1:{port}/")
                    else None,
                )
                held = []

                def hold_old_status(route):
                    if not held:
                        held.append(route)
                    else:
                        route.continue_()

                page.route("**/api/v1/admin/status", hold_old_status)
                page.goto(f"http://127.0.0.1:{port}/admin")
                page.wait_for_function('window.KagNodeCore?.version.startsWith("5.0.")')
                expect(page.locator("[data-core-login]")).to_be_visible()
                expect(page.locator(".kn-core-login-provider")).to_be_hidden()
                assert page.locator("main:visible").count() == 1
                assert (
                    page.locator(".kn-core-login-card").evaluate(
                        "(e)=>getComputedStyle(e).borderRadius"
                    )
                    == "12px"
                )
                assert (
                    page.locator("#credential").evaluate(
                        "(e)=>getComputedStyle(e).borderRadius"
                    )
                    == "8px"
                )
                frame = page.locator("[data-core-login]").evaluate("""e=>{
                  const b=e.getBoundingClientRect(),s=getComputedStyle(e);
                  return {x:b.x,width:b.width,height:b.height,paddingLeft:parseFloat(s.paddingLeft),maxWidth:s.maxWidth,margin:s.margin,background:s.backgroundColor,canonicalBackground:getComputedStyle(document.documentElement).getPropertyValue('--kn-panel').trim(),viewport:innerWidth,viewportHeight:innerHeight};
                }""")
                assert (
                    abs(frame["x"]) < 1 and abs(frame["width"] - frame["viewport"]) < 1
                ), frame
                assert frame["height"] >= frame["viewportHeight"] - 1, frame
                assert frame["paddingLeft"] == (12 if width <= 480 else 20), frame
                assert frame["maxWidth"] == "none" and frame["margin"] == "0px", frame
                assert frame["background"] not in ["transparent", "rgba(0, 0, 0, 0)"], (
                    frame
                )
                assert page.locator("[data-core-login]").evaluate(
                    "e=>{const n=document.createElement('div');n.style.backgroundColor='var(--kn-panel)';e.append(n);const color=getComputedStyle(n).backgroundColor;n.remove();return getComputedStyle(e).backgroundColor===color}"
                ), frame
                expect(page.locator("#login-message")).to_be_hidden()
                page.screenshot(path=str(artifacts / f"{width}-login.png"))
                page.locator("#credential").fill(credential)
                page.get_by_role("button", name="Sign in", exact=True).click()
                expect(page.locator("#admin-panel")).to_be_visible()
                assert page.locator("main:visible").count() == 1
                assert len(held) == 1
                held[0].fulfill(
                    status=401,
                    content_type="application/json",
                    body='{"error":{"message":"old session expired"}}',
                )
                page.wait_for_timeout(100)
                expect(page.locator("#admin-panel")).to_be_visible()
                page.unroute("**/api/v1/admin/status", hold_old_status)
                if width < 1024:
                    page.locator(".kn-core-mobile-toggle").click()
                    expect(page.locator("#sidebar")).to_be_in_viewport()
                    page.keyboard.press("Escape")
                else:
                    for _ in range(3):
                        page.evaluate("KagNodeCore.navigation.collapse(true)")
                        page.evaluate("KagNodeCore.navigation.collapse(false)")
                assert page.locator("#sidebar [data-core-icon] svg").count() == 4
                for link in page.locator("#sidebar nav a").all():
                    assert link.locator("svg").count() == 1
                assert page.locator("#sidebar #scroll-mode").count() == 0
                scrolling = page.get_by_role(
                    "combobox", name="Scrolling on this browser", exact=True
                )
                scrolling.click()
                page.get_by_role("option", name="Native scrolling", exact=True).click()
                assert page.evaluate("KagNodeScroll.mode") == "native"
                scrolling.click()
                page.get_by_role("option", name="KagNode scrolling", exact=True).click()
                assert page.evaluate("KagNodeScroll.mode") == "custom"
                client = f"fictional-{width}"
                page.locator("#client-id").fill(client)
                page.locator("#confirm-key").fill(credential)
                page.get_by_role("button", name="Apply change").click()
                expect(page.locator("#clients")).to_contain_text(client)
                token = page.locator("#new-token").inner_text()
                assert len(token) == 43
                page.get_by_role("button", name="Clear credential", exact=True).click()
                expect(page.locator("#new-token")).to_be_empty()
                expect(page.locator("#message")).to_have_text("Client access updated.")
                assert page.evaluate(
                    "document.documentElement.scrollWidth <= innerWidth+1"
                )
                page.screenshot(path=str(artifacts / f"{width}-dashboard.png"))
                page.get_by_role("button", name="Open account menu", exact=True).click()
                page.get_by_role("button", name="Sign out", exact=True).click()
                expect(page.locator("#admin-login")).to_be_visible()
                expect(page.locator("#admin-panel")).to_be_hidden()
                assert not errors, errors
                assert not external, external
                results.append(
                    {
                        "width": width,
                        "canonicalLogin": True,
                        "clientManagement": True,
                        "localAssetsOnly": True,
                        "errors": 0,
                    }
                )
                context.close()
            browser.close()
            print(json.dumps({"engine": kind, "journeys": results}))
    finally:
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
