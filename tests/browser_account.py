"""Explicit fictional issuer/Axum integration; never use production accounts."""

import asyncio, json, re, html, http.client, os
from pathlib import Path
from urllib.parse import urlsplit, parse_qs, urljoin
from playwright.async_api import async_playwright

W = Path(os.environ["ENTROPY_BROWSER_OUTPUT"])
W.mkdir(mode=0o700, parents=True, exist_ok=True)


async def main():
    local = json.loads(Path(os.environ["ENTROPY_TEST_STATE"]).read_text())
    issuer = json.loads(Path(os.environ["KAGNODE_TEST_ISSUER_STATE"]).read_text())
    origin = local["origin"]
    assert (
        urlsplit(origin).hostname == urlsplit(issuer["origin"]).hostname == "127.0.0.1"
    )
    assert urlsplit(origin).scheme == urlsplit(issuer["origin"]).scheme == "http"
    assert issuer["issuer"] == "https://nebulacoder.com"
    assert issuer["user"]["email"].endswith((".test", ".invalid"))
    external = []
    errors = []
    callbacks = []
    responses = []
    starts = []
    delayed = []
    async with async_playwright() as p:
        options = {"headless": True}
        executable = os.environ.get("KAGNODE_BROWSER_EXECUTABLE") or os.environ.get(
            "PLAYWRIGHT_CHROMIUM_EXECUTABLE"
        )
        options.update({"executable_path": executable} if executable else {})
        b = await getattr(
            p, os.environ.get("KAGNODE_BROWSER_ENGINE", "chromium")
        ).launch(**options)
        c = await b.new_context(viewport={"width": 390, "height": 900})
        page = await c.new_page()
        page.set_default_timeout(30000)
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on(
            "response",
            lambda r: responses.append(
                {"path": urlsplit(r.url).path, "status": r.status}
            ),
        )

        async def guard(route):
            url = route.request.url
            if urlsplit(url).hostname == "nebulacoder.com":
                target = (
                    issuer["origin"]
                    + urlsplit(url).path
                    + ("?" + urlsplit(url).query if urlsplit(url).query else "")
                )
                headers = await route.request.all_headers()
                headers.pop("host", None)
                if route.request.is_navigation_request():
                    headers["sec-fetch-dest"] = "document"
                headers.pop("accept-encoding", None)
                headers["connection"] = "close"
                jar = await c.cookies("https://nebulacoder.com")
                headers["cookie"] = "; ".join(v["name"] + "=" + v["value"] for v in jar)

                def relay():
                    parsed = urlsplit(target)
                    conn = http.client.HTTPConnection(
                        parsed.hostname, parsed.port, timeout=15
                    )
                    try:
                        conn.request(
                            route.request.method,
                            parsed.path + ("?" + parsed.query if parsed.query else ""),
                            body=route.request.post_data_buffer,
                            headers=headers,
                        )
                        r = conn.getresponse()
                        body = r.read()
                        pairs = r.getheaders()
                        result = {
                            k.lower(): v
                            for k, v in pairs
                            if k.lower()
                            not in ["content-length", "transfer-encoding", "connection"]
                        }
                        cookies = [v for k, v in pairs if k.lower() == "set-cookie"]
                        if cookies:
                            result["set-cookie"] = "\n".join(cookies)
                        return r.status, result, body
                    finally:
                        conn.close()

                status, forwarded, body = await asyncio.to_thread(relay)
                if status in [301, 302, 303, 307, 308]:
                    destination = urljoin(url, forwarded["location"])
                    assert urlsplit(
                        destination
                    ).hostname == "nebulacoder.com" or destination.startswith(
                        origin + "/auth/kagnode/callback?"
                    )
                    mirrored = {
                        "content-type": "text/html; charset=utf-8",
                        "cache-control": "no-store",
                    }
                    if "set-cookie" in forwarded:
                        mirrored["set-cookie"] = forwarded["set-cookie"]
                    await route.fulfill(
                        status=200,
                        headers=mirrored,
                        body='<!doctype html><meta http-equiv="refresh" content="0;url='
                        + html.escape(destination, quote=True)
                        + '"><title>Continuing isolated sign-in</title>',
                    )
                else:
                    await route.fulfill(status=status, headers=forwarded, body=body)
            elif url.startswith(origin + "/"):
                if "/auth/kagnode/callback?" in url:
                    callbacks.append(url)
                if (
                    urlsplit(url).path == "/api/v1/admin/status"
                    and len(callbacks) >= 2
                    and not delayed
                ):
                    response = await route.fetch()
                    if response.status == 401:
                        delayed.append(True)
                        await asyncio.sleep(1)
                    await route.fulfill(response=response)
                elif urlsplit(url).path == "/api/v1/admin/kagnode/start":
                    response = await route.fetch()
                    starts.append(await response.json())
                    await route.fulfill(response=response)
                else:
                    await route.continue_()
            else:
                external.append(urlsplit(url).hostname)
                await route.abort()

        await c.route("**/*", guard)
        await page.goto(origin + "/admin")
        await page.locator("#credential").fill(local["credential"])
        await page.locator("#admin-login-form button[type=submit]").click()
        await page.locator("#admin-panel").wait_for(state="visible")
        await page.goto(origin + "/admin/link")
        await page.locator("#credential").fill(local["credential"])
        await page.locator("[data-core-login-provider]").click()
        await page.wait_for_url("https://nebulacoder.com/**")
        started = starts[-1]
        assert "user_code" not in started and "authorization_uri" in started
        await page.wait_for_url("https://nebulacoder.com/**")
        await page.wait_for_function(
            """()=>!!document.querySelector('form[action="/login"]')||[...document.querySelectorAll("button")].some(b=>/approve|allow|authorize/i.test(b.textContent))"""
        )
        if await page.locator('form[action="/login"]').count():
            form = page.locator('form[action="/login"]')
            hidden = await form.locator("[name=next]").input_value()
            assert urlsplit(hidden).path == "/oauth/authorize"
            await form.locator("[name=email]").fill(issuer["user"]["email"])
            next = form.get_by_role("button", name="Continue", exact=True)
            if await next.count() and await next.is_visible():
                await next.click()
            await form.locator("[name=password]").fill(issuer["user"]["password"])
            await form.locator("button[type=submit]").click()
        await page.wait_for_url("https://nebulacoder.com/**")
        approve = page.locator("button[name=decision][value=approve]")
        await approve.first.wait_for(state="visible")
        await approve.first.click()
        await page.wait_for_url(origin + "/admin**")
        await page.locator("#admin-panel").wait_for(state="visible")
        assert "kagnode_attempt" not in page.url
        store = json.loads((Path(local["directory"]) / "credentials.json").read_text())
        assert store["owner"]["subject"] == issuer["user"]["id"]
        assert store["account_enabled"] is True
        # Replayed callback cannot rotate or create a session.
        replay = await page.request.get(callbacks[-1], max_redirects=0)
        assert replay.status == 410
        original_digest = store["admin_digest"]
        subject = issuer["user"]["id"]

        async def begin_signin():
            await page.locator("[data-core-account-trigger]").first.click()
            await page.locator("#signout").click()
            await page.locator("#admin-login").wait_for(state="visible")
            await page.locator("[data-core-login-provider]").click()
            await page.wait_for_url("https://nebulacoder.com/**")
            await page.locator("button[name=decision][value=approve]").wait_for(
                state="visible"
            )
            return starts[-1]

        again = await begin_signin()
        await page.locator("button[name=decision][value=approve]").click()
        await page.wait_for_url(origin + "/admin**")
        await page.locator("#admin-panel").wait_for(state="visible")
        await page.wait_for_timeout(1300)
        assert delayed and await page.locator("#admin-panel").is_visible()
        denied = await begin_signin()
        await page.wait_for_timeout(1200)
        pending = await page.request.post(
            origin + "/api/v1/admin/kagnode/poll",
            data={"attempt_id": denied["attempt_id"]},
            headers={"Origin": origin},
        )
        assert pending.status == 200 and (await pending.json())["pending"] is True
        await page.locator("button[name=decision]:not([value=approve])").click()
        await page.wait_for_url(origin + "/admin**")
        await (
            page.locator("[data-core-login-status]")
            .filter(has_text="not approved")
            .wait_for(state="visible")
        )
        assert await page.locator("#admin-panel").is_hidden()
        await page.locator("[data-core-login-provider]").click()
        await page.wait_for_url("https://nebulacoder.com/**")
        await page.locator("button[name=decision][value=approve]").wait_for(
            state="visible"
        )
        cancelled = starts[-1]
        cancellation = await page.request.post(
            origin + "/api/v1/admin/kagnode/cancel",
            data={"attempt_id": cancelled["attempt_id"]},
            headers={"Origin": origin},
        )
        assert (
            cancellation.status == 200
            and (await cancellation.json())["cancelled"] is True
        )
        gone = await page.request.post(
            origin + "/api/v1/admin/kagnode/poll",
            data={"attempt_id": cancelled["attempt_id"]},
            headers={"Origin": origin},
        )
        assert gone.status == 410
        await page.goto(origin + "/admin")
        await page.locator("#credential").fill(local["credential"])
        await page.locator("#admin-login-form button[type=submit]").click()
        await page.locator("#admin-panel").wait_for(state="visible")
        store = json.loads((Path(local["directory"]) / "credentials.json").read_text())
        assert (
            store["owner"]["subject"] == subject
            and store["admin_digest"] == original_digest
        )
        assert callbacks and parse_qs(urlsplit(callbacks[-1]).query)["iss"] == [
            "https://nebulacoder.com"
        ]
        assert not errors and not external, (errors, external)
        (W / "actual-automatic-report.json").write_text(
            json.dumps(
                {
                    "actualIssuer": True,
                    "actualAxum": True,
                    "explicitLocalKeyBeforeLink": True,
                    "sameTabAuthorization": True,
                    "pkceCodeExchangeAndExactOwnerSubject": True,
                    "noDeviceCode": True,
                    "callbackIssuerPresent": True,
                    "lateAnonymous401CannotClearNewSession": True,
                    "repeatProviderSignIn": True,
                    "delayedConsentPending": True,
                    "deniedShowsUsefulError": True,
                    "cancelConsumesAttempt": True,
                    "replayRejected": True,
                    "localKeyRemainsUsable": True,
                    "automaticUIResume": True,
                    "core": "5.0.5",
                    "external": external,
                    "pageErrors": errors,
                },
                indent=2,
            )
            + "\n"
        )
        print(
            "PASS actual issuer/Axum automatic sign-in, owner linking, denial, cancellation, delayed status, replay and local fallback"
        )
        await c.close()
        await b.close()


asyncio.run(main())
