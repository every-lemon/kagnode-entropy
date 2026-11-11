"""Real API/UI failure path with a deliberately absent local decoder.
The missing path is checked before opening HTTP, so no stream is requested.
"""

import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import tempfile
from playwright.sync_api import sync_playwright, expect

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="kagnode-entropy-fixture-") as fixture:
    missing = str(Path(fixture) / "deliberately-absent-decoder")
    source = "https://video.example.invalid/fictional-media.mp4"
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    process = subprocess.Popen(
        [
            os.environ.get(
                "ENTROPY_BINARY", str(root / "target/debug/kagnode-entropy")
            ),
            "serve",
            "--ephemeral",
            "--port",
            str(port),
            "--mode",
            "experimental",
            "--acknowledge-experimental",
            "--video-url",
            source,
            "--video-decoder",
            missing,
            "--video-retries",
            "0",
        ],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        token = None
        for _ in range(4):
            found = re.search(
                r"preview-alice: ([A-Za-z0-9_-]+)", process.stdout.readline()
            )
            if found:
                token = found.group(1)
                break
        assert token
        with sync_playwright() as p:
            options = {"headless": True}
            if os.environ.get(
                "KAGNODE_BROWSER_ENGINE", os.environ.get("BROWSER", "chromium")
            ) == "chromium" and os.environ.get("PLAYWRIGHT_CHROMIUM_EXECUTABLE"):
                options["executable_path"] = os.environ[
                    "PLAYWRIGHT_CHROMIUM_EXECUTABLE"
                ]
            if os.environ.get("KAGNODE_BROWSER_EXECUTABLE"):
                options["executable_path"] = os.environ["KAGNODE_BROWSER_EXECUTABLE"]
            browser = getattr(
                p,
                os.environ.get(
                    "KAGNODE_BROWSER_ENGINE", os.environ.get("BROWSER", "chromium")
                ),
            ).launch(**options)
            page = browser.new_page(viewport={"width": 390, "height": 844})
            page.goto(f"http://127.0.0.1:{port}/")
            page.locator("#token").fill(token)
            page.get_by_role("button", name="Connect", exact=True).click()
            page.locator("#dashboard").wait_for(state="visible")
            expect(page.locator("#source-state")).to_contain_text("Source failed")
            expect(page.locator("#consume")).to_be_disabled()
            headers = {"Authorization": "Bearer " + token}
            response = page.request.get(
                f"http://127.0.0.1:{port}/api/v1/status", headers=headers
            )
            assert response.status == 200
            status = response.json()
            assert status["producer_fault"] is True
            assert status["source_status"]["kind"] == "public-video"
            assert status["source_status"]["state"] == "failed"
            assert status["source_status"]["frames"] == 0
            assert status["source_status"]["last_error"] == "video-configuration"
            assert source not in response.text() and missing not in response.text()
            response = page.request.post(
                f"http://127.0.0.1:{port}/api/v1/bytes",
                headers=headers,
                data={"count": 1},
            )
            assert (
                response.status == 503
                and response.json()["error"]["code"] == "unavailable"
            )
            assert (
                page.request.get(f"http://127.0.0.1:{port}/api/v1/status").status == 401
            )
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            browser.close()
    finally:
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        assert process.returncode == 1, (
            "A failed source must leave a nonzero exit status"
        )
print(
    "Source failure QA passed: configured video never fetched, failed UI remains available, allocations blocked, authentication enforced, no private URL/path in status."
)
