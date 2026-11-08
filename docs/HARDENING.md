# Optional audit and defensive policy

## Metadata ledger

Auditing requires an explicit key file and a **new** session directory. The CLI's `audit-key` command creates a new 32-byte OS-random key with mode 0600. On Linux, key reads reject symlinks, non-regular files, incorrect size, wrong ownership and group/other permissions. Session directories are created with mode 0700 and records with mode 0600. Existing directories and keys are never overwritten.

Each record contains a format version, global event index, previous record MAC and a typed metadata event. HMAC-SHA-512 covers the canonical serialized payload using an audit-specific label and a separate audit key. Generated chunks, final outputs, engine keys, tokens, client identities, contexts, raw frames and raw counter text are never audit payload fields.

Default events contain only produced sequence or allocated first sequence/count. Optional flags add:

| Flag | Additional metadata | Limit |
| --- | --- | --- |
| `--audit-timing` | Measured processing duration in nanosecond units | Timer precision and execution variation are not entropy estimates. |
| `--audit-network` | Aggregate absolute RX/TX byte totals if counter input parses | No interface names; totals can still reveal workload patterns. Use `--linux-counters` separately to provide real host counters. |
| `--audit-frame-fingerprint` | Keyed pixel fingerprint and available geometry | Can correlate repeated frames within a key scope; does not prove source integrity or entropy. |

There is no sub-nanosecond CPU-cycle recording, forensic proof-of-entropy claim or automatic privacy-safe classification. Enable detailed metadata only for a justified purpose.

## Bounds, retention and verification

The default is four segments, each holding at most 1,024 records. Allowed bounds are 2–32 retained segments and 1–10,000 records per segment, with at most 2,048 bytes per record. Rotation removes only the oldest segment created by this running instance; it never scans or cleans unrelated files. A temporary extra empty segment can exist during rotation; filesystem metadata and crash leftovers are outside the record-byte bound. Repeated sessions in new directories require operator-managed retention.

Every append is flushed with `sync_data`. This requests durability from the OS; it is not a power-loss or hardware guarantee. An I/O failure disables further audit writes and prevents successful output delivery from that engine. A removed allocation is burned on error.

Graceful termination prints a checkpoint with the retained first index, final index and final MAC. Keep checkpoints in an independently authenticated location and protect the audit key separately. The preview does not upload or notarize them. For example, use an independently recorded final index and head:

```sh
kagnode-entropy audit-verify --directory /path/to/audit-session-1 \
  --key-file /path/to/audit.key \
  --expected-index 42 --expected-head '<independently-retained-128-hex-head>'
```

Verification checks MACs and index/chain continuity in the retained records against that final checkpoint. It does not prove deleted prefix history, honest telemetry or delivery to a consumer. Prefix deletion is visible through the reported first retained index but requires a separately trusted retention expectation to assess. An attacker who has the key can generate alternative chains; an attacker controlling the host can suppress logging. Adjacent files are not independent trust anchors. There is no `chattr +i` dependency or claim of a non-falsifiable/compliance-certified vault.

## Tracer policy and shutdown

`serve --stop-if-traced` checks only `/proc/self/status` before serving and every 250 ms. A positive or unreadable/malformed `TracerPid` result stops this instance. It does not inspect other processes, modify ptrace policy, kill unrelated processes, change kernel settings or attempt to fight an administrator.

Polling can miss attachment between checks, and a hostile host can spoof or suppress it. A debugger may read state before the check. SIGINT/SIGTERM request ordinary shutdown: owned key/queue buffers are cleared and connection tasks are stopped. Existing outputs cannot be recalled. SIGKILL, a crash, kernel failure and power loss cannot guarantee cleanup; `zeroize` does not clear every possible copy.

Audit and tracer behavior are tested with fictional temporary data and parser fixtures. No real debugger attachment, raw-memory probe, physical-erasure test or hardware qualification was performed.
