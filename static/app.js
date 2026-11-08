'use strict';
const $ = selector => document.querySelector(selector);
let token = '', busy = false, generation = 0;
async function api(path, options = {}) {
  const response = await fetch('/api/v1/' + path, {cache: 'no-store', ...options,
    headers: {'Authorization': 'Bearer ' + token, ...(options.body ? {'Content-Type':'application/json'} : {})}});
  const body = await response.json();
  if (!response.ok) throw new Error(body.error?.message || 'Request failed');
  return body;
}
async function refresh() {
  if (!token || document.hidden) return;
  const current = generation;
  const status = await api('status');
  if (current !== generation || !token) return;
  $('#client-name').textContent = status.client;
  $('#mode').textContent = status.mode === 'os-backed' ? 'OS-backed mode' : 'Experimental · unassessed';
  $('#queued').textContent = status.queued;
  $('#capacity').textContent = `${status.capacity} chunks maximum · 64 bytes each`;
  $('#produced').textContent = status.produced;
  $('#consumed').textContent = status.consumed;
  $('#counter-source').textContent = status.counter_source === 'linux-proc-net-dev'
    ? 'This instance uses bounded host counters from /proc/net/dev. They add no claimed entropy.'
    : 'This instance uses fictional counters. No host traffic is read.';
  $('#pool-fill').max = status.capacity;
  $('#pool-fill').value = status.queued;
  $('#engine-note').textContent = status.producer_fault || status.harvester_failure
    ? 'The producer stopped after a source failure. Restart only after checking the local adapter.'
    : status.mode === 'experimental'
      ? 'Experimental output is for pipeline research. The supplemental inputs have no assessed entropy.'
      : 'Fresh OS random bytes are mixed into every generation. The synthetic inputs add no claimed entropy.';
  $('#consume').disabled = busy || status.producer_fault || status.queued < 1;
  $('#dashboard').hidden = false;
  $('#connected').hidden = false;
  $('#connect-form').hidden = true;
  $('#connection-status').textContent = 'Connected to the local source preview.';
}
$('#connect-form').addEventListener('submit', async event => {
  event.preventDefault();
  const button = event.currentTarget.querySelector('button');
  button.disabled = true;
  generation++;
  token = $('#token').value.trim();
  $('#token').value = '';
  try { await refresh(); }
  catch (error) { token = ''; $('#connection-status').textContent = error.message; }
  finally { button.disabled = false; }
});
$('#disconnect').addEventListener('click', () => {
  generation++;
  token = '';
  $('#output').textContent = 'No output allocated.';
  $('#dashboard').hidden = true;
  $('#connected').hidden = true;
  $('#connect-form').hidden = false;
  $('#connection-status').textContent = 'Disconnected. The token is no longer retained by this page.';
  $('#token').focus();
});
$('#consume').addEventListener('click', async () => {
  if (busy) return;
  const current = generation;
  busy = true;
  $('#consume').disabled = true;
  $('#output-status').textContent = 'Allocating…';
  try {
    const result = await api('bytes', {method:'POST', body:JSON.stringify({count:1, context:$('#context').value})});
    if (current !== generation || !token) return;
    const block = result.blocks[0];
    $('#output').textContent = block.base64;
    $('#output-status').textContent = `${result.client} · allocation ${block.sequence} · ${block.bytes} bytes`;
  } catch (error) { if (current === generation && token) $('#output-status').textContent = error.message; }
  finally {
    busy = false;
    if (current !== generation || !token) return;
    try { await refresh(); } catch { $('#connection-status').textContent = 'Connection unavailable. No allocation is retried automatically.'; }
  }
});
$('#clear').addEventListener('click', () => {
  $('#output').textContent = 'No output allocated.';
  $('#output-status').textContent = '';
});
setInterval(() => refresh().catch(() => { $('#connection-status').textContent = 'Connection unavailable. No allocation is retried automatically.'; }), 4000);
