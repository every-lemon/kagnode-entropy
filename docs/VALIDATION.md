# Verification — 0.1.0-alpha.3

The published primary runtime is Rust. The temporary Python prototype is not part of this repository. Tests use deterministic fixtures, temporary fake audit records and isolated loopback HTTP. No public video source, camera, native hardware entropy device, real account, live mail service or enclave deployment is involved.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
python3 -m pip install playwright==1.64.1
python3 -m playwright install chromium
python3 tests/browser_check.py
python3 tests/browser_failure_check.py
```

Locally verified on Linux x86-64 using Rust 1.99.1:

- **64 Rust tests passed**, including RFC HMAC/HKDF known-answer vectors, blueprint order, independent key/chunk material, deterministic replay, source failure without fallback, bounded inputs, atomic batches, concurrent allocation/production, fork refusal and owned-buffer cleanup.
- Audit checks passed for bounded retention, opt-in metadata, tampering, truncation against an independent checkpoint, wrong keys, symlinks, existing paths, sticky I/O failure and burned allocations on audit failure.
- API checks passed for identity binding, client rate isolation and out-of-order timestamp handling, body/origin/Host/method limits, local asset boundaries and public-bind refusal. The product-artwork inventory test verifies bundled bytes against pinned digests and retained notices; the prior Core stylesheet and legacy mark are preserved.
- Video checks use fake HTTP, deterministic frames and a tiny RGB-emitting process stub. They cover public-address/DNS/redirect boundaries, pipe-only decoder arguments, resource limits, byte/frame/time bounds, partial-frame discard, retries, cancellation/reaping, renewal and exact frame-to-HMAC input. They also cover RFC 9637 documentation-prefix boundaries and cleanup after an engine snapshot failure.
- SGX/Nitro **mock** policy checks passed for platform mismatch, debug/SVN rejection, nonce/key/freshness checks and the unconditional refusal to claim hardware verification or release secrets.
- CLI checks cover rejection of invalid video URLs before creating an audit session and graceful SIGTERM cancellation during source retry. The optimized alpha.2 binary passed 1/16 exact 64-byte output checks, the experimental acknowledgement gate and temporary audit/checkpoint verification. The alpha.3 optimized rebuild additionally passed its reported-version and two-output smoke checks.
- Chromium checks passed at 1,365, 390 and 320 CSS pixels in dark/light themes: allocation, disconnection, late-response suppression, no horizontal overflow or persistent token storage. Anonymous Chromium confirmed the published GitHub README’s light/dark logo references decode and render; a narrow-screen check reproduced its fixed-height distortion, now covered by a README markup regression that preserves the artwork aspect ratio under GitHub’s width constraint. The local product lockup loaded correctly in both themes, and bundled 32px/180px PNGs were checked for transparent canvases. The disposable server exited cleanly on SIGTERM. Regressions cover Clear output winning against an in-flight response, context/attribution clearing on disconnect useful feedback for non-JSON HTTP failures, reconnecting while an old allocation is pending, and late poll errors after disconnect. A missing-decoder fixture verifies the real failed-source API/UI, blocked allocations, authentication and URL/path privacy without opening a network connection.

The source declares Rust 1.88 as its minimum language/dependency target; local qualification used 1.99.1. There are no prebuilt binaries, published Cargo package or supported production deployment in this release. Dependencies are pinned in `Cargo.lock`; the Core stylesheet hash is pinned separately.

These checks establish specified behavior. They do not measure min-entropy, certify the construction, prove collision impossibility, demonstrate physical erasure, validate genuine attestation or benchmark production throughput. No real FFmpeg process, live stream, independent cryptographic review or platform qualification was involved. Main publication uses required GitHub Actions checks and pull requests with administrator enforcement. Browser screenshots and transient development artifacts stay under ignored `.runtime/`.
