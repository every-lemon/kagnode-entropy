'use strict';
const $ = selector => document.querySelector(selector);
let token = '', busy = false, generation = 0, outputGeneration = 0, statusGeneration = 0;
const requests = new Set();
function cancelRequests() {
  for (const request of requests) request.abort();
  requests.clear();
}
async function api(path, options = {}) {
  const controller = new AbortController();
  requests.add(controller);
  try {
    const response = await fetch('/api/v1/' + path, {cache: 'no-store', ...options, signal: controller.signal,
      headers: {'Authorization': 'Bearer ' + token, ...(options.body ? {'Content-Type':'application/json'} : {})}});
    let body;
    try { body = await response.json(); }
    catch { throw new Error(`The local server returned an unexpected response (${response.status}). No allocation was retried.`); }
    if (!response.ok) throw new Error(body.error?.message || `Request failed (${response.status}).`);
    return body;
  } finally { requests.delete(controller); }
}
async function refresh() {
  if (!token || document.hidden) return;
  const current = generation;
  const request = ++statusGeneration;
  let status;
  try { status = await api('status'); }
  catch (error) {
    if (current !== generation || request !== statusGeneration || !token) return;
    throw error;
  }
  if (current !== generation || request !== statusGeneration || !token) return;
  $('#client-name').textContent = status.client;
  $('#mode').textContent = status.mode === 'os-backed' ? 'OS-backed mode' : 'Experimental · unassessed';
  $('#queued').textContent = status.queued;
  $('#capacity').textContent = `${status.capacity} chunks maximum · 64 bytes each`;
  $('#produced').textContent = status.produced;
  $('#consumed').textContent = status.consumed;
  $('#counter-source').textContent = status.counter_source === 'linux-proc-net-dev'
    ? 'This instance uses bounded host counters from /proc/net/dev. They add no claimed entropy.'
    : 'This instance uses fictional counters. No host traffic is read.';
  const source = status.source_status;
  const video = source?.kind === 'public-video';
  $('#pixel-source').textContent = video
    ? `Configured for ${source.geometry[0]} × ${source.geometry[1]} RGB frames from the administrator’s public video source. No entropy estimate is assigned.`
    : 'A predictable 16 × 16 RGB fixture. No live stream is connected.';
  const sourceStates = {idle:'Waiting to start', connecting:'Connecting', running:'Running', backpressure:'Pool full · input paused', retrying:'Reconnecting within the retry budget', restarting:'Renewing the bounded decoder session', failed:'Source failed · allocations blocked', stopped:'Stopped'};
  const sourceErrors = {'video-configuration':'The source or decoder configuration was rejected.', 'video-network':'The media source could not be reached.', 'video-input-limit':'The encoded input byte limit was reached.', 'decoder-start':'The configured decoder could not start.', 'video-ended':'The media stream ended.', 'frame-truncated':'The decoder returned an incomplete frame.', 'frame-timeout':'No complete frame arrived before the deadline.', 'decoder-cleanup':'Decoder cleanup did not complete; further starts are blocked.', 'engine-failure':'The engine rejected the generation.'};
  $('#source-state').textContent = `${video ? 'Public video' : 'Synthetic input'} · ${sourceStates[source?.state] || 'Status unavailable'}${source?.last_error ? '. ' + (sourceErrors[source.last_error] || 'Inspect the local source configuration.') : ''}`;
  $('#source-state').classList.toggle('source-failed', Boolean(status.producer_fault));
  $('#pool-fill').max = status.capacity;
  $('#pool-fill').value = status.queued;
  $('#engine-note').textContent = status.producer_fault || status.harvester_failure
    ? source?.state === 'retrying'
      ? 'The source is retrying within its configured limit. Allocations are paused until a complete frame is accepted.'
      : 'The producer stopped after a source failure. Restart only after checking the local adapter.'
    : status.mode === 'experimental'
      ? 'Experimental output is for pipeline research. The supplemental inputs have no assessed entropy.'
      : 'Fresh OS random bytes are mixed into every generation. Supplemental inputs add no claimed entropy.';
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
  const current = generation;
  cancelRequests();
  busy = false;
  token = $('#token').value.trim();
  $('#token').value = '';
  try { await refresh(); }
  catch (error) { if (current === generation) { token = ''; $('#connection-status').textContent = error.message; } }
  finally { if (current === generation) button.disabled = false; }
});
$('#disconnect').addEventListener('click', () => {
  generation++;
  outputGeneration++;
  statusGeneration++;
  cancelRequests();
  busy = false;
  token = '';
  $('#context').value = '';
  $('#output-status').textContent = '';
  $('#client-name').textContent = '';
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
  const display = ++outputGeneration;
  busy = true;
  $('#consume').disabled = true;
  $('#output-status').textContent = 'Allocating…';
  try {
    const result = await api('bytes', {method:'POST', body:JSON.stringify({count:1, context:$('#context').value})});
    if (current !== generation || display !== outputGeneration || !token) return;
    const block = result.blocks[0];
    $('#output').textContent = block.base64;
    $('#output-status').textContent = `${result.client} · allocation ${block.sequence} · ${block.bytes} bytes`;
  } catch (error) { if (current === generation && display === outputGeneration && token) $('#output-status').textContent = error.message; }
  finally {
    if (current !== generation || !token) return;
    busy = false;
    try { await refresh(); } catch { $('#connection-status').textContent = 'Connection unavailable. No allocation is retried automatically.'; }
  }
});
$('#clear').addEventListener('click', () => {
  outputGeneration++;
  $('#output').textContent = 'No output allocated.';
  $('#output-status').textContent = '';
});
setInterval(() => refresh().catch(() => { $('#connection-status').textContent = 'Connection unavailable. No allocation is retried automatically.'; }), 4000);
