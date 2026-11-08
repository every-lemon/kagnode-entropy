# Local preview API

`kagnode-entropy serve` listens only on IPv4 loopback, default port 8098. It is not a public production server or a deployed KagNode cloud service. All API responses use `Cache-Control: no-store`; CORS is not enabled. Host must match the local URL, and any supplied Origin must match that same HTTP origin. Query strings and transfer encoding are rejected.

## Authentication

The CLI prints two disposable bearer tokens, each bound to one fixed identity. Send `Authorization: Bearer TOKEN`. Do not put tokens in URLs, source repositories, logs or browser persistent storage. Client identity cannot be supplied in request JSON. Restart invalidates all tokens. There is no token-management or account-integration service in this preview.

`GET /health` is public and identifies source-preview status and the absence of trusted execution. It is a liveness response, not proof that a source can supply frames. Static assets are public on the loopback listener.

`GET /api/v1/status` requires authentication and returns the client's identity, mode, capacity, queued/produced/consumed counts, producer failure state, audit-enabled state, zero claimed supplemental entropy bits and `trusted_execution: "none: ordinary process"`. It also returns `source_status` with kind/state, geometry, accepted-frame/start/failure counts and a fixed error code. It never returns secret key, S3, queue material, source frames, configured source URL, decoder path or audit key. Exhausted video failures keep status available while allocations return 503.

## Allocate

`POST /api/v1/bytes` requires authentication, `Content-Type: application/json`, an exact `Content-Length` and a body of at most 1,024 bytes:

```json
{"count":1,"context":"example-session"}
```

Only `count` (1–16, default 1) and `context` (at most 128 UTF-8 bytes, default empty) are accepted. A context is an application label, not a credential or entropy source. The response contains `client`, `mode`, and `blocks`; each block has an instance ID, sequence, `bytes:64` and base64 output.

A batch is allocated under one lock. Insufficient pool capacity removes nothing. Once removed, a chunk is never requeued; an ambiguous client timeout or lost response must not cause automatic retry. A retry is a new allocation, not recovery of an earlier result.

Per-client allocation budget is 16 chunks/second with a burst of 32. Failed empty-pool attempts also consume that rate budget. HTTP service permits at most eight simultaneous connections, a five-second header deadline and ten-second total connection lifetime; keep-alive is disabled.

| Status | Meaning |
| --- | --- |
| 400 | Invalid fields, body, count, context or transfer encoding |
| 401 | Missing or invalid bearer token |
| 403 | Invalid Host/Origin or query-string request |
| 404 / 405 | Unknown path / unsupported method |
| 413 / 415 | Body too large / incorrect content type |
| 429 | Per-client allocation budget exhausted; Retry-After: 1 |
| 503 | Pool empty, engine/source/audit failure; Retry-After: 1 |

Framework-level protocol errors may return a plain error body. Do not assume every unsuccessful response is JSON. APIs do not expose audit detail, raw telemetry, platform attestation or remote source configuration.
