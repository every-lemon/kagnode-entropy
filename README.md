<p align="center"><img src="static/mark.svg" width="72" height="72" alt="KagNode mark"></p>
<h1 align="center">KagNode Entropy</h1>
<p align="center">A Rust source preview for OS-backed randomness and supplemental-input research.</p>

**Version 0.1.0-alpha.1 · AGPL-3.0-only · Linux · local preview**

KagNode Entropy mixes bounded raw-pixel samples, timestamps and counter bytes through a three-stage HMAC-SHA-512 pipeline. A locked queue allocates each queued chunk once, and authenticated client identities bind the final output. Separate HKDF labels derive the next key and queued material.

The default **OS-backed** mode adds fresh operating-system randomness on every generation. The **experimental** mode omits that fresh contribution and requires explicit acknowledgement. Neither mode assigns an entropy estimate to video, timing or network counters. For ordinary application secrets, use the OS RNG directly; this experimental construction has not received an independent cryptographic review.

The included interface runs locally, serves KagNode Core styles without a CDN and uses synthetic pixels by default. This is an ordinary Rust process. SGX and Nitro are separate proposed deployment profiles, with simulation-only policy tests; no enclave service or verified hardware attestation is included.

## Run locally

Install a current [Rust toolchain](https://rust-lang.org/tools/install/) on Linux. Build with the committed lockfile:

```sh
cargo build --locked --release
./target/release/kagnode-entropy sample --count 2
./target/release/kagnode-entropy serve
```

Open `http://127.0.0.1:8098`. The terminal prints two disposable local client tokens once. Paste one into the interface; it stays in page memory and is cleared on disconnect. Restarting replaces the tokens and the engine state. Do not save terminal output containing tokens or redirect generated samples into an audit log.

```sh
# Explicit research mode, not an assessed entropy source:
./target/release/kagnode-entropy serve --mode experimental --acknowledge-experimental

# Optional Linux counter input; the default uses fictional counters:
./target/release/kagnode-entropy serve --linux-counters
```

No video service is contacted. An application adapter can pass decoded frames to the bounded `Sample` API; see [design and integration](docs/DESIGN.md). A real decoder, public-video connector and source health assessment remain future work.

## Included

- Two explicit modes, a 64-byte key, fresh OS input in the default mode and separate rekey/output derivation.
- Atomic bounded allocations, client-bound output, per-client rate limits and bounded HTTP connections.
- A locally served, responsive KagNode interface with clear preview state.
- An opt-in metadata-only HMAC-chained audit with bounded retention and private files.
- Optional own-process tracer detection that stops this instance and clears owned buffers on a best-effort basis.
- SGX/Nitro design profiles and mock policy checks that **never** authorize secret release.
- Known-answer, concurrency, bounds, failure, audit, API and policy tests.

## Audit and defensive controls

Auditing is **off by default**. It never records generated outputs, current keys, raw pixels, raw counter blobs, client identities, tokens or context strings. Timing, aggregate network totals and keyed frame fingerprints are separate privacy opt-ins.

```sh
mkdir -p "$HOME/.local/state/kagnode/entropy"
./target/release/kagnode-entropy audit-key \
  --path "$HOME/.local/state/kagnode/entropy/audit.key"
./target/release/kagnode-entropy serve \
  --audit-key-file "$HOME/.local/state/kagnode/entropy/audit.key" \
  --audit-directory "$HOME/.local/state/kagnode/entropy/audit-session-1" \
  --audit-segment-records 1024 --audit-retained-segments 4
```

The key file must be new; each session directory must also be new. The default retention cap is four segments of at most 1,024 records each. Graceful shutdown prints a checkpoint to retain independently. An attacker with the audit key can forge a chain; a checkpoint stored only beside the log cannot establish independent history. Read [audit and defense](docs/HARDENING.md) before relying on it.

`serve --stop-if-traced` opts into polling this process's Linux `TracerPid` field. This can interrupt legitimate debugging and can miss or be bypassed by an attacker. There is no `chattr`, firewall, ptrace, kernel, VM or host-security modification. SIGKILL and power loss cannot run cleanup handlers.

## Review and contribute

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Read [threat model](docs/THREAT-MODEL.md), [API](docs/API.md), [TEE roadmap](docs/TEE.md), [validation](docs/VALIDATION.md) and [security reporting](SECURITY.md).

Source code is licensed under **AGPL-3.0-only**. KagNode artwork has separate [artwork terms](static/ARTWORK-LICENSE). The locally bundled Core stylesheet has [provenance](static/core/NOTICE) and a [pinned digest](core.lock.json). The UI uses system fonts.

[KagNode Website](https://nebulacoder.com) · [Developer](https://developer.nebulacoder.com) · [Source](https://github.com/every-lemon/kagnode-entropy)
