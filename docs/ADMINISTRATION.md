# Local administration

`init --directory NEW_DIRECTORY` refuses existing paths and writes a private configuration plus `admin.key`. Read the key only on the trusted host. Never run the service with its private directory inside a public asset directory or source repository. Run as a dedicated unprivileged service user; existing files must remain owned by that user with no group/other access.

`serve --credentials-file DIRECTORY/credentials.json` loads persistent credential hashes. The listener is loopback-only, validates Host/Origin, rejects query credentials and does not support arbitrary reverse-proxy origins. The separate `--ephemeral` option is for disposable development and prints temporary tokens; do not use it in unattended service logs.

## Client access

Sign in at `/admin` with the local administrator key. Create a client ID, confirm the key and save the new credential shown once. Rotate replaces its old hash; revoke removes it. Credentials permit the client status and allocation APIs, never administration. Each identity has a 16-block/second refill rate and 32-block burst; requests contain 1–16 blocks. A block is 64 bytes. Pool availability can impose lower throughput. Rotation does not recall previously delivered data.

The server limits credential records, sessions, pending account attempts, administrative event history, HTTP connection count, body sizes and timeouts. Configuration writes are serialized, atomically replaced and synced. A process-held exclusive file lease prevents two servers from racing the same directory. Do not externally edit the live credential store.

## KagNode owner linking

Sign in locally, open Account and choose Connect or change KagNode Account. Re-enter the local key, then approve the exact displayed origin/code at KagNode. The server requests `profile.read email.read` with an unpredictable browser-bound nonce. It checks the first-exchange authorization's issuer, client, installation, purpose, nonce, immutable subject and time bounds, and checks the retrieved profile's matching identity.

Only that explicitly linked subject can use Login with KagNode. No public registration, email matching or remote administrator role is accepted. Display name, approved profile photo and explicitly consented verified contact email update on successful provider sign-in; they do not replace the local recovery credential. Server-to-server grants remain out of the browser and are revoked after retrieving the sign-in profile; this adapter does not persist refresh credentials or synchronize continuously in the background.

Disable integration or disconnect the owner in Account by confirming the local key. Pending attempts and existing admin sessions are invalidated; local sign-in still works. `KAGNODE_ACCOUNT_SYNC=off` disables provider use at the process level. Cancellation consumes the local attempt; an interrupted or ambiguous exchange requires starting again. Account linking never grants consumer API credentials.

## API boundaries

`POST /api/v1/admin/login` exchanges the local key for a 15-minute HttpOnly/SameSite=Strict cookie and a CSRF value. Loopback HTTP cookies intentionally have no Secure flag; these are not internet-facing cookies. Browser writes use `X-CSRF-Token`; client-key changes and account-policy changes additionally require the local key. `GET /api/v1/admin/status` returns aggregate engine health, client IDs and bounded sanitized current-process events, never credential hashes or raw outputs.

`POST /api/v1/admin/clients` supports create, rotate and revoke. `POST /api/v1/admin/logout` invalidates the session and pending owner sign-ins. Provider interfaces are under `/api/v1/admin/kagnode/`: availability, start, poll, cancel, settings and policy. Device secrets remain in bounded server memory, expire after ten minutes and cannot survive restart. The Core controller handles code presentation, polling, cancellation and terminal feedback.

Administrator-key recovery requires trusted local access to `admin.key`; preserve it separately from public source. Do not delete the store to change client access. Loss or suspected disclosure of the local key requires taking this instance offline and a deliberate local credential recovery procedure; no unauthenticated web reset exists.

## Interrupted configuration writes

A failure before atomic replacement leaves the prior configuration active. If replacement occurred but directory durability could not be confirmed, the process blocks all new consumer authorizations and administrative sessions. It does not retry a mutation against stale in-memory permissions. Stop it, inspect the private configuration and retained backup, then deliberately restart after resolving storage health. Provider operations have a six-second total budget within the HTTP/browser limits; timeout or cancellation invalidates their local attempt. Deadlines are checked again before owner/session finalization.
