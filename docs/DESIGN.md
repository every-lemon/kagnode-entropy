# Engine design — 0.1.0-alpha.1

## Data flow

A `Sample` holds a zeroizing owned raw-pixel buffer, unsigned 128-bit nanosecond timestamp, zeroizing counter byte buffer and optional validated `[width,height,channels]` geometry. Limits are 1 MiB of pixels and 16 KiB of counters. Synthetic RGB samples are 16×16×3. Nanosecond units do not establish nanosecond clock accuracy or entropy.

Given the old 64-byte key `K`, the blueprint stages are preserved:

```text
S1 = HMAC-SHA512(K, pixels)
S2 = HMAC-SHA512(S1, timestamp_as_16_big_endian_bytes)
S3 = HMAC-SHA512(S2, counter_bytes)
```

S3 remains internal. It is neither directly queued nor installed as the next key. In OS-backed mode `fresh` is 64 bytes from `getrandom`; in experimental mode it is empty. Both modes require a 64-byte OS seed and a separate 16-byte instance identifier at startup.

```text
D = UTF8("KagNode Entropy/source-preview/v1/")
F(fields...) = concatenation of uint64_be(byte_length) || field for each field
IKM = F(D, UTF8(mode), fresh, S3, uint64_be(sequence))
PRK = HKDF-Extract-SHA512(salt=K, IKM)
chunk = HKDF-Expand-SHA512(PRK, D || "pool-chunk", 64)
next_key = HKDF-Expand-SHA512(PRK, D || "next-key", 64)
output = HMAC-SHA512(chunk,
  F(D, "client-output", authenticated_client_id, context,
    lowercase_hex_instance_id, uint64_be(sequence)))
```

The RustCrypto crates implement the cryptographic primitives. Different labels provide domain separation; they do not prove mathematical uniqueness or certify this composition. The OS RNG remains the default security source. There is no weak fallback after an RNG failure.

## Atomicity and lifecycle

One mutex covers key rotation, queue mutation, sequence counters and audit ordering. A full queue returns backpressure before sampling more OS random bytes. Batch allocation validates the whole requested size before removing anything. Every removed chunk is burned even if writing the response or audit record fails. No automatic allocation retry is performed.

A source failure blocks allocation. The CLI stops the instance; library users may explicitly retry generation after repairing a source. An audit write error is sticky for that audit instance. Successful generation audit records precede queue insertion; a record is a diagnostic event, not proof of successful delivery or assessed entropy.

A process-ID check occurs before taking the mutex. Forked copies must construct a fresh engine. The preview does not prevent VM rollback, restored snapshots, cloned environments or a privileged adversary. Instance and sequence IDs identify allocations within an uninterrupted instance; they do not assert collision impossibility across machines.

## Limits

| Resource | Preview bound |
| --- | --- |
| Queue | 1–4,096 × 64-byte chunks; default 128 |
| Allocation batch | 1–16 chunks |
| Authenticated identities | 1–64; CLI issues 2 |
| Identity/context | 128 UTF-8 bytes in engine; registry IDs max 64 |
| API body | 1,024 bytes; no transfer encoding |
| Per-client allocation budget | 16 chunks/second; burst 32 |
| HTTP connections | 8; 5-second header / 10-second total timeout |
| Producer tick | 20–60,000 ms; default 100 ms; skipped missed ticks |
| Optional audit | 2–32 segments × 1–10,000 records; max 2,048 bytes/record |

The local preview intentionally bounds throughput. No sustained high-speed or production throughput claim has been measured. Synchronous audit flushes add latency while holding the engine lock.

## Adapter boundary

Library callers supply decoded pixels via `Sample`; source ingestion never interprets a URL or launches a decoder. A future video adapter must bound resolution, frame rate, decoder memory, queue time and subprocess lifetime. A public stream is observable and influenceable. Authenticating transport or fingerprinting pixels does not assess its entropy.

The CLI defaults to fictional counters and synthetic frames. `--linux-counters` reads only the fixed `/proc/net/dev` path, with a bounded read. No arbitrary filesystem scan or shell command is used. A future enclave adapter needs a separate RNG/I/O integration and validation; ordinary Linux `/proc` and OS RNG assumptions cannot simply be relabeled enclave-trusted.

## Memory

Owned secret arrays, chunks and sample buffers use `zeroize::Zeroizing`; the old current-key buffer is explicitly overwritten on rotation and shutdown. Crate internal state, compiler-created copies, registers, response strings, kernel/socket buffers, browser memory, swap and crash dumps are outside this guarantee. Dropping a queue item does not instantly remove every physical copy. See [the zeroize project's guarantees and limitations](https://docs.rs/zeroize/latest/zeroize/).
