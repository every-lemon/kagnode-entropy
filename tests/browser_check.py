"""Disposable local Rust preview QA. No live service, mail or real credential input."""
import os
import hashlib
import json
from pathlib import Path
import re
import signal
import socket
import subprocess
import time
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[1]
BRAND = {item['path']: item['sha256'] for item in json.loads((ROOT / 'branding.lock.json').read_text())['files']}
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
    bob = re.search(r'preview-bob: ([A-Za-z0-9_-]+)', process.stdout.readline()).group(1)
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
            expect(page.locator('header .brand img')).to_have_attribute('alt', 'KagNode. Entropy')
            assert page.locator('header .brand img').evaluate('(image) => image.complete && image.naturalWidth > 0')
            assert page.locator('header .brand img').evaluate('(image) => image.currentSrc').endswith(f'entropy-lockup-{scheme}.svg')
            assert page.locator('link[rel=icon][type="image/svg+xml"]').get_attribute('href') == '/brand/favicon.svg'
            for asset in ('favicon.svg', 'favicon.png', 'favicon.ico', 'entropy-mark-512.png'):
                response = page.request.get(f'http://127.0.0.1:{port}/brand/{asset}')
                assert response.status == 200
                assert hashlib.sha256(response.body()).hexdigest() == BRAND['static/brand/' + asset]
            alpha = page.evaluate("""() => new Promise((resolve,reject) => {
              const image = new Image(); image.onerror = reject;
              image.onload = () => {const canvas = document.createElement('canvas');
                canvas.width = image.width; canvas.height = image.height;
                const ctx = canvas.getContext('2d'); ctx.drawImage(image, 0, 0);
                resolve({width:image.width, height:image.height, corner:ctx.getImageData(0,0,1,1).data[3]});};
              image.src = '/brand/favicon.png';
            })""")
            assert alpha == {'width': 32, 'height': 32, 'corner': 0}
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
            page.locator('#context').fill('private-fixture-context')
            page.get_by_role('button', name='Disconnect').click()
            assert page.locator('#dashboard').is_hidden()
            assert page.locator('#output').inner_text() == 'No output allocated.'
            assert page.locator('#context').input_value() == ''
            assert page.locator('#output-status').inner_text() == ''
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
        # Clear output also wins against a delayed response in the same identity.
        context = browser.new_context()
        page = context.new_page()
        page.goto(f'http://127.0.0.1:{port}/')
        page.locator('#token').fill(token)
        page.get_by_role('button', name='Connect', exact=True).click()
        page.locator('#dashboard').wait_for(state='visible')
        page.evaluate("""() => {
          const realFetch = window.fetch;
          window.fetch = async (...args) => {
            const response = await realFetch(...args);
            if (String(args[0]).endsWith('/bytes')) await new Promise(resolve => setTimeout(resolve, 500));
            return response;
          };
        }""")
        page.get_by_role('button', name='Allocate 64 bytes').click()
        page.get_by_role('button', name='Clear output').click()
        page.wait_for_timeout(700)
        assert page.locator('#output').inner_text() == 'No output allocated.'
        assert page.locator('#output-status').inner_text() == ''
        # A framework/proxy plain-text error gets actionable feedback, no retry.
        requests = []
        def plain_error(route):
            requests.append(1)
            route.fulfill(status=503, content_type='text/plain', body='fixture failure')
        page.route('**/api/v1/bytes', plain_error)
        page.get_by_role('button', name='Allocate 64 bytes').click()
        expect(page.locator('#output-status')).to_contain_text('unexpected response (503)')
        assert len(requests) == 1
        context.close()
        # Pending work belongs to its original session, even if a transport ignores abort.
        context = browser.new_context()
        page = context.new_page()
        page.add_init_script('window.setInterval = fn => {window.fixturePoll = fn; return 1}')
        page.goto(f'http://127.0.0.1:{port}/')
        page.locator('#token').fill(token)
        page.get_by_role('button', name='Connect', exact=True).click()
        page.locator('#dashboard').wait_for(state='visible')
        # An older status response cannot roll the dashboard back.
        page.evaluate("""() => {
          window.fixtureFetch = fetch; window.statusSequence = 0; window.pendingStatus = [];
          window.fetch = async (...args) => {
            const id = ++window.statusSequence;
            const response = await window.fixtureFetch(...args);
            const body = await response.json(); body.produced = 200 + id;
            return new Promise(resolve => {window.pendingStatus[id - 1] = () => resolve(new Response(JSON.stringify(body), {status: 200, headers: {'Content-Type':'application/json'}}));});
          };
          window.fixturePoll(); window.fixturePoll();
        }""")
        page.wait_for_function('() => typeof pendingStatus[0] === "function" && typeof pendingStatus[1] === "function"')
        page.evaluate('pendingStatus[1]()')
        expect(page.locator('#produced')).to_have_text('202')
        page.evaluate('pendingStatus[0]()')
        page.wait_for_timeout(100)
        expect(page.locator('#produced')).to_have_text('202')
        page.evaluate('window.fetch = window.fixtureFetch')
        page.evaluate("""() => {
          const real = fetch; window.pendingAllocations = [];
          window.fetch = async (...args) => {
            const response = await real(...args);
            if (String(args[0]).endsWith('/bytes')) await new Promise(resolve => window.pendingAllocations.push(resolve));
            return response;
          };
        }""")
        page.locator('#consume').click()
        page.wait_for_function('() => pendingAllocations.length === 1')
        page.locator('#disconnect').click()
        page.locator('#token').fill(bob)
        page.get_by_role('button', name='Connect', exact=True).click()
        page.locator('#dashboard').wait_for(state='visible')
        expect(page.locator('#client-name')).to_have_text('preview-bob')
        expect(page.locator('#consume')).to_be_enabled(timeout=1000)
        page.locator('#consume').click()
        page.wait_for_function('() => pendingAllocations.length === 2')
        page.evaluate('pendingAllocations[0]()')
        page.wait_for_timeout(100)
        expect(page.locator('#consume')).to_be_disabled()
        expect(page.locator('#output')).to_have_text('No output allocated.')
        page.evaluate('pendingAllocations[1]()')
        expect(page.locator('#output-status')).to_contain_text('preview-bob')
        expect(page.locator('#consume')).to_be_enabled()
        # A stale poll error cannot replace the disconnected status.
        page.evaluate("""() => {
          window.fetch = () => new Promise((resolve, reject) => window.rejectStatus = reject);
          window.fixturePoll();
        }""")
        page.wait_for_function('() => typeof rejectStatus === "function"')
        page.locator('#disconnect').click()
        page.evaluate('rejectStatus(new Error("fictional transport failure"))')
        page.wait_for_timeout(100)
        expect(page.locator('#connection-status')).to_contain_text('Disconnected.')
        assert page.locator('#dashboard').is_hidden()
        context.close()
        browser.close()
    print('Browser QA passed: desktop/mobile/small, both themes, canonical logo/favicon bytes, allocation, session replacement, ordered status, late failures, no overflow or persistent token storage.')
finally:
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
    assert process.returncode == 0, 'Preview did not shut down cleanly'
