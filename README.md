<p align="center"><a href="#brand-artwork"><picture><source media="(prefers-color-scheme: dark)" srcset="static/brand/entropy-lockup-dark.svg"><img src="static/brand/entropy-lockup-light.svg" width="304" alt="KagNode. Entropy"></picture></a></p>
<h1 align="center">KagNode Entropy</h1>
<p align="center">A local Rust service for operating-system cryptographic randomness.</p>

## Overview

KagNode Entropy distributes bounded, unchanged 64-byte blocks from the OS cryptographic RNG. Production output does not pass through a custom video/timing mixer. A memory-bounded queue and atomic allocation keep individual queue entries from being allocated twice within an uninterrupted instance; they do not promise mathematically unique byte strings or protection against host snapshots.

The locally served web interface provides a client API explorer and a separate administrator console. KagNode Core supplies the login frame, sidebar, icons, controls and scrolling. Browsers never fetch Core assets from GitHub.

## Release status

**5.0.2** is the coordinated stable source patch, updating shared controls and adding an explicit reviewed local installer. Linux · AGPL-3.0-only.

Stable scope is the OS-backed local service, its API and administration. Supplemental-input experiments remain explicitly experimental. Video/decoder, enclave and hardware qualification are outside this release. No entropy-source assessment, cryptographic certification or compromised-host protection is claimed.

## Features

Core 5.0.6 supplies the shared controls and account-photo presentation. This embedded service has no automatic UI update manager and reports update summaries as unavailable. Its reviewed installer builds and starts only the local OS-backed service.

- OS CSPRNG output, bounded atomic batches, per-client quotas and source-failure handling.
- Private persistent client credential hashes, one-time token display, rotation and revocation.
- Separate short-lived administrator sessions, CSRF protection and credential confirmation for access changes.
- Explicit KagNode Account owner linking and fresh sign-in approval; no signup, email matching or remote role assignment.
- Core 5 login, responsive navigation, profile display and account-only appearance controls.
- Optional bounded metadata audit; generated output and credentials are excluded.
- Offline startup and local administrator-key sign-in when KagNode is unavailable.

## Requirements

Linux; Rust/Cargo 1.88 or newer and the TLS dependencies required by the locked Rust dependencies. Build verification targets Linux x86-64. OpenSSL, Python 3 and rustfmt are needed to regenerate the verified UI. Browser development checks use Playwright with Chromium and WebKit.

The service deliberately listens only on loopback. Run browser administration on the trusted host or through an SSH tunnel to its canonical local URL. Internet-facing hosting and reverse-proxy deployment are not supported by this release's origin policy.

## Installation and downloads

| Download | Contents | Platform |
| --- | --- | --- |
| Source `.tar.xz`, `.tar.gz`, `.zip` | Complete allowlisted source, embedded UI and instructions | Build on supported Linux |
| Source `.deb` | Source/documentation under `/usr/share/kagnode/kagnode-entropy-source` | Architecture-independent; no service activation |
| Source `.rpm` | Same source/documentation | Rocky Linux 10 package-installation scope; no native provisioning |

Verify `SHA256SUMS` and `release-manifest.json` from the release. Source packages do not install a runnable daemon or reconfigure an existing host.

```sh
git clone https://github.com/every-lemon/kagnode-entropy.git
cd kagnode-entropy
cargo build --locked --release
```

## Usage

Create a **new** private configuration directory. Its parent must already exist:

```sh
./target/release/kagnode-entropy init --directory /path/to/new-private-directory
./target/release/kagnode-entropy serve --credentials-file /path/to/new-private-directory/credentials.json
```

Open `http://127.0.0.1:8098/admin`. Read `admin.key` locally and sign in; do not paste it into issues, terminal recordings or chat. Create a client credential and save its one-time value securely. `/` provides a client explorer; `/api/v1/bytes` allocates output using a consumer bearer credential. Administrator credentials do not authorize that consumer API.

Use `sample --count 2` for local output. Disposable development uses `serve --ephemeral`; only that explicit mode prints temporary client tokens. Production startup does not print secrets.

## Configuration and storage

The administrator directory is owned by the service user with mode 0700; files use 0600. `credentials.json` contains token hashes and the explicitly linked owner's display profile, not generated randomness or plaintext consumer keys. `admin.key` is a sensitive local recovery credential. Keep the directory outside the checkout/public assets and protect its backups. One process owns the store at a time; updates use atomic replacement and synchronization.

Administrator sessions and recent administrative events are bounded and memory-only. Sessions expire after 15 minutes and disappear on restart. The queue also starts fresh. Client hashes and owner settings persist. Local sign-in remains available when account synchronization is disabled. `KAGNODE_ACCOUNT_SYNC=off` also disables provider sign-in.

Optional metadata audit remains separate; see [audit configuration](docs/HARDENING.md). Core assets are embedded and pinned; updating the Rust application requires a reviewed rebuild. This release does not claim an automatic Rust Core adapter updater.

## Security

Read [SECURITY.md](SECURITY.md) and the [threat model](docs/THREAT-MODEL.md). The service trusts the OS RNG, host, compiler and dependencies. API transport and remote client storage add boundaries beyond using the OS RNG directly in an application.

No random outputs, bearer credentials, raw media or account tokens belong in logs or issues. Token revocation blocks subsequent authorizations; it cannot recall already allocated or in-flight responses. Discard the output of ambiguous allocation attempts rather than automatically treating a retry as the same request.

## Development and testing

```sh
python3 scripts/build_ui.py
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
python3 tests/browser_check.py
python3 tests/browser_failure_check.py
python3 tests/browser_admin.py
```

`ui/` contains editable host pages; `static/admin.js`, `static/link.js` and `static/app.js` contain application interactions. `scripts/build_ui.py` verifies the signed pinned Core manifest and builds `static/*.html` and `src/core_assets.rs`. Shared component source and canonical templates are retained in `vendor/core/`; make shared component changes in Core first. Do not hand-edit generated or minified Core assets.

Tests use fictional accounts, deterministic RNGs, mocked issuer responses and isolated temporary directories. Decoder fixtures do not run real FFmpeg or fetch live media. See [validation scope](docs/VALIDATION.md).

## Documentation

- [Reviewed local setup](docs/INSTALLER.md)

[Administration and account linking](docs/ADMINISTRATION.md) · [API](docs/API.md) · [Engine design](docs/DESIGN.md) · [Audit](docs/HARDENING.md) · [Experimental video](docs/VIDEO.md) · [Threat model](docs/THREAT-MODEL.md) · [TEE research](docs/TEE.md)

<a id="brand-artwork"></a>
[SVG symbol](static/brand/entropy-mark-light.svg) · [Transparent PNG](static/brand/entropy-mark-512.png) · [Canonical artwork collection](https://github.com/every-lemon/kagnode-logos)

## Licensing

Software is **AGPL-3.0-only**; see [LICENSE](LICENSE). Product artwork retains its [separate terms](static/ARTWORK-LICENSE), [pinned provenance](branding.lock.json) and [font notices](static/brand/FONT-NOTICES.txt). Imported Core source and artwork retain [license](vendor/core/LICENSE.txt), [artwork terms](vendor/core/ARTWORK-LICENSE.txt) and [version lock](core.lock.json).
