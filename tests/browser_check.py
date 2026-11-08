"""Disposable local Rust preview QA. No live service, mail or real credential input."""
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import time
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / '.runtime' / 'browser'
ARTIFACTS.mkdir(parents=True, exist_ok=True)
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
process = subprocess.Popen([str(ROOT / 'target/debug/kagnode-entropy'), 'serve', '--port', str(port)],
                           cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
try:
    token = None
    for _ in range(4):
        line = process.stdout.readline()
        match = re.search(r'preview-alice: ([A-Za-z0-9_-]+)', line)
        if match:
            token = match.group(1)
            break
    assert token, 'Disposable preview token was not issued'
    with sync_playwright() as p:
        options = {'headless': True}
        if os.environ.get('PLAYWRIGHT_CHROMIUM_EXECUTABLE'):
            options['executable_path'] = os.environ['PLAYWRIGHT_CHROMIUM_EXECUTABLE']
        browser = p.chromium.launch(**options)
        for name, width, height, scheme in [('desktop', 1365, 1100, 'dark'), ('mobile', 390, 844, 'light'), ('small', 320, 740, 'dark')]:
            context = browser.new_context(viewport={'width': width, 'height': height}, color_scheme=scheme)
            page = context.new_page()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.goto(f'http://127.0.0.1:{port}/')
            page.locator('#token').fill(token)
            page.get_by_role('button', name='Connect', exact=True).click()
            page.locator('#dashboard').wait_for(state='visible')
            page.get_by_role('button', name='Allocate 64 bytes').click()
            expect(page.locator('#output')).to_have_text(re.compile(r'^[A-Za-z0-9+/]{86}==$'))
            assert page.locator('#client-name').inner_text() == 'preview-alice'
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            assert page.evaluate('localStorage.length === 0 && sessionStorage.length === 0')
            assert not errors, errors
            page.screenshot(path=str(ARTIFACTS / f'{name}.png'), full_page=True)
            page.get_by_role('button', name='Disconnect').click()
            assert page.locator('#dashboard').is_hidden()
            assert page.locator('#output').inner_text() == 'No output allocated.'
            context.close()
        # A late response from the disconnected identity must not restore output.
        context = browser.new_context()
        page = context.new_page()
        page.goto(f'http://127.0.0.1:{port}/')
        page.locator('#token').fill(token)
        page.get_by_role('button', name='Connect', exact=True).click()
        page.locator('#dashboard').wait_for(state='visible')
        page.evaluate('''() => {
          const realFetch = window.fetch;
          window.fetch = async (...args) => {
            const response = await realFetch(...args);
            if (String(args[0]).endsWith('/bytes')) await new Promise(resolve => setTimeout(resolve, 500));
            return response;
          };
        }''')
        page.get_by_role('button', name='Allocate 64 bytes').click()
        page.get_by_role('button', name='Disconnect').click()
        page.wait_for_timeout(700)
        assert page.locator('#dashboard').is_hidden()
        assert page.locator('#output').inner_text() == 'No output allocated.'
        context.close()
        browser.close()
    print('Browser QA passed: desktop/mobile/small, both themes, allocation, disconnect, late response, no overflow or persistent token storage.')
finally:
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
    assert process.returncode == 0, 'Preview did not shut down cleanly'
