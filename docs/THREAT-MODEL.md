# Threat model

This source preview is intended for inspection and pipeline experiments. Its downstream security purpose has not been specified. It is not an independently reviewed DRBG, entropy appliance, cloud service or compliance-certified product.

## Trust and adversaries

OS-backed mode trusts the host OS, its RNG, the Rust toolchain, dependencies and this implementation. Assume an adversary can observe or influence public video, timing and network traffic, call the local API without credentials, race consumers or exhaust request budgets. Do not assume the supplemental observations are independent.

The opt-in video connector introduces an HTTP/TLS and multimedia-decoder attack surface. Protocol restrictions, process limits and child cleanup are not a security sandbox; real decoder/host isolation is unqualified.

The ordinary process cannot protect keys against a compromised kernel, root, a debugger with access, hostile hypervisor, stolen live-memory image or malicious build dependency. A caller that controls the injected library `RandomSource` also controls the RNG assumption. Injection exists for deterministic tests and explicit integrators; the CLI always uses the OS source.

## Claims the preview deliberately does not make

| Proposed claim | Supported statement |
| --- | --- |
| Avoid all PRNG predictability | OS-backed mode relies on the OS CSPRNG; experimental mixing is not a substitute for an assessed source. |
| HMAC creates entropy from public video | Hashing can condition inputs; it does not establish their unpredictability. Claimed supplemental entropy is zero bits. |
| Absolute historical security after full RAM compromise | Independent rekey/output labels avoid exposing the next key as output, but this composition has no such proof. Queued chunks and in-flight outputs remain sensitive. |
| `popleft()` instantly physically deletes data | Allocation removes one owned queue entry. Selected owned buffers are zeroized; copies, hardware and downstream buffers are not comprehensively erased. |
| Impossible collisions or reuse everywhere | An uninterrupted engine allocates each sequence once; cryptographic collisions are not mathematically impossible, and snapshots/rollback can reproduce state. |
| Client ID guarantees unique isolated environments | IDs bind authenticated application contexts. They do not supply entropy or replace process/host isolation. |
| Timing or frame metadata proves entropy | Diagnostics describe observed values, not min-entropy, independence, authenticity or source health. |
| Immutable root-proof compliance vault | The optional HMAC ledger is tamper-evident only relative to trusted keys and independently retained checkpoints. Root can delete evidence, suppress events or forge it after key compromise. |
| Debugger detection gives instant key protection | Opt-in polling is racy and bypassable. Debuggers can access memory before the next check, and legitimate administration may trigger it. |
| SIGKILL or power failure invokes secure cleanup | These events cannot run application cleanup. No absolute erasure or cold-boot guarantee is claimed. |
| SGX and Nitro eliminate all privileged attacks | They have different trust boundaries. Neither is integrated or hardware-validated here; see the separate [TEE roadmap](TEE.md). |

## Remaining work before production consideration

Specify the consuming application's security needs; obtain independent cryptographic and implementation review; assess actual sources and real-decoder behavior; implement source-specific health testing; independently review decoder boundaries, token lifecycle, TLS, quotas and multi-process ownership; validate restore/rollback behavior and an incident-response policy. If an enclave target is selected, build and verify that separate target and client attestation protocol before releasing secrets to it.

## Primary references

[NIST SP 800-90B](https://csrc.nist.gov/pubs/sp/800/90/b/final) defines assessment and health-testing requirements for entropy sources. This preview has not performed that assessment. Statistical appearance alone does not establish entropy.

[NIST SP 800-90A Rev. 1](https://csrc.nist.gov/pubs/sp/800/90/a/r1/final) specifies DRBG constructions. This project does not claim to implement a validated 800-90A DRBG.

[RFC 5869](https://www.rfc-editor.org/rfc/rfc5869) describes HKDF extract/expand and context information. The project uses separate labels; that does not independently validate the surrounding engine.

[Linux getrandom](https://man7.org/linux/man-pages/man2/getrandom.2.html) and the [getrandom crate](https://docs.rs/getrandom/latest/getrandom/) describe the OS randomness interface and platform assumptions. Initialization can block; source errors do not invoke an insecure fallback.

[RFC 4231](https://www.rfc-editor.org/rfc/rfc4231) supplies HMAC known-answer vectors used in tests. Passing a vector checks integration, not the full threat model.
