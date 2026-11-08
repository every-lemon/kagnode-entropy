# Public-video connector — 0.1.0-alpha.2

The Rust runtime now includes an administrator opt-in connector for **direct HTTP(S) media streams**. Synthetic pixels remain the default. A configured source supplies decoded raw RGB pixels to the existing HMAC stages; it does not supply assessed entropy or verified physical randomness.

```sh
# Replace this reserved example hostname with a public media source you may use.
kagnode-entropy serve \
  --video-url 'https://media.example.invalid/live.ts' \
  --video-decoder /usr/bin/ffmpeg \
  --video-width 64 --video-height 64 --video-fps 10
```

An administrator must install and maintain the decoder separately. There is no automatic download or installation. The command also works with `sample`; all source settings are local command-line settings, never browser/API parameters.

## Supported input

Use a direct, streamable media resource such as MPEG-TS, progressive MP4, WebM or another explicitly allowed container supported by the local decoder. A YouTube/watch-page URL, HTML page, HLS/DASH playlist, local file, camera, authentication-protected stream or arbitrary decoder protocol is not supported. Some MP4 files require seeking and cannot be decoded from a pipe; a format name alone does not guarantee compatibility.

Rust fetches encoded media and feeds the child through stdin. FFmpeg receives `pipe:0`, a fixed demuxer allowlist, fixed first-video-stream selection, an FPS/scale filter, RGB24 rawvideo and `pipe:1`. It receives no URL, custom argument list, shell command, interactive stdin, report setting, remote path or inherited proxy/credential environment. Audio, subtitles and data streams are excluded.

The transport accepts public HTTP(S) hosts on ports 80/443. Credentials in URLs, fragments, local names, private/loopback/link-local and listed special-use addresses and mixed public/private DNS answers are rejected. The resolver returns the checked socket addresses directly to the HTTP client. Redirects are limited and revalidated, and HTTPS cannot redirect to HTTP. Certificate validation stays enabled; proxy inheritance, cookies, authorization headers, automatic request retries and TLS-key logging are absent/disabled.

HTTP transport has no entropy or authenticity guarantee beyond its selected transport. Plain HTTP remains observable and modifiable. A source can manipulate media, redirect to a different public source or repeat frames. Use only sources you are authorized to access, and avoid secret-bearing URL query strings; the administrator's CLI arguments can be visible to local process inspection.

## Bounded operation

| Control | Default | Allowed |
| --- | --- | --- |
| Raw RGB width / height | 64 / 64 | 1–512 each; engine limit 1 MiB/frame |
| Output frame rate | 10 fps | 1–30 fps |
| Frame/start timeout | 10 seconds | 1–30 seconds via CLI |
| Decoder session lifetime | 300 seconds | 1–600 seconds |
| Encoded bytes per transfer | 64 MiB | 1–256 MiB |
| Child virtual-address-space limit | 512 MiB | 128–2,048 MiB |
| Failure retries per source instance | 2 | 0–5; one-second delay |
| Child CPU / descriptors | 120 CPU seconds / 64 | Fixed per decoder process |

The decoder uses one worker thread per configured decode/filter/output stage. Child-only limits set zero core/file-size limits and cap virtual address space, CPU time and file descriptors. These controls are **not a security sandbox**: a decoder vulnerability remains part of the host threat model. Use a maintained decoder under an appropriately isolated service account before considering deployment.

A full pool does not start a source or read another frame. Backpressure propagates through bounded OS pipes; no complete media download or unbounded frame queue is accumulated. Network bytes are counted before being written into the decoder. A partial frame is discarded, and a timeout/cancellation stops the owned decoder before further attempts. After a complete frame, the clock is sampled and then optional network counters are read.

Normal frame/lifetime limits, and a reached transfer budget after valid frames, renew a session after its previous decoder has been stopped and reaped. An oversized initial response or a source that never yields a complete frame uses the finite failure budget. EOF, malformed/partial frames, network failure and unexpected decoder exit do not silently switch to synthetic input. Restarting a finite media file can repeat frames; this contributes no additional assessed entropy.

On a source failure, allocations pause while the finite retry budget is used. A subsequent successful generation clears that source fault. Once retries are exhausted, the local interface remains available for diagnosis, allocations stay blocked and the process ultimately exits nonzero when stopped. A cleanup/reaping failure prevents another decoder start. Shutdown kills the owned process group and awaits the child; abnormal OS failure, SIGKILL or power loss cannot guarantee application cleanup.

Timers are runtime deadlines, not hard real-time guarantees under a stalled kernel or scheduler. The parent process and HTTP/TLS libraries still use their own bounded working buffers; the child address-space limit is not a total host-memory claim.

## Status and privacy

Authenticated status exposes only source kind/state, geometry, accepted-frame/start/failure counts and a fixed error code. It never returns the configured URL, decoder path, raw frames, encoded media or decoder stderr. The audit continues to use its typed metadata schema; raw media and outputs never become audit payload fields. Optional frame fingerprints remain correlatable metadata, not provenance or entropy proofs.

## What was tested

Tests use fictional HTTP transport, deterministic RGB sessions and a small local stub process that emits three fixed bytes. They verify decoding-pipe arguments, inherited-environment removal, child resource limits, process cleanup, frame boundaries, cancellation, backpressure, retries, renewal, public-address/redirect rules and exact frame-to-HMAC integration. A deliberately missing decoder exercises the real CLI/API/UI failure path without opening a network connection.

**No real FFmpeg process or live media source was executed during these checks.** Real-codec compatibility, source-specific behavior, throughput and hostile-media qualification remain unverified. This is implemented source-preview functionality, not a qualified production video service.

The address policy also rejects the expanded IPv6 documentation range [3fff::/20 defined by RFC 9637](https://www.rfc-editor.org/rfc/rfc9637.html). It is not a source for real public traffic.

Primary references: [FFmpeg protocol options](https://ffmpeg.org/ffmpeg-protocols.html), [FFmpeg common options](https://ffmpeg.org/ffmpeg-all.html), [Tokio child lifecycle](https://docs.rs/tokio/latest/tokio/process/struct.Command.html), [reqwest custom DNS resolution](https://docs.rs/reqwest/latest/reqwest/dns/trait.Resolve.html).
