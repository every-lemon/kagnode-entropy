# Local administration

`init --directory NEW_DIRECTORY` refuses existing paths and writes a private configuration plus `admin.key`. Read the key only on the trusted host. Never run the service with its private directory inside a public asset directory or source repository. Run as a dedicated unprivileged service user; existing files must remain owned by that user with no group/other access.

`serve --credentials-file DIRECTORY/credentials.json` loads persistent credential hashes. The listener is loopback-only, validates Host/Origin, rejects query credentials except the bounded OAuth callback and opaque local resume ID and does not support arbitrary reverse-proxy origins. The separate `--ephemeral` option is for disposable development and prints temporary tokens; do not use it in unattended service logs.

## Client access

Sign in at `/admin` with the local administrator key. Create a client ID, confirm the key and save the new credential shown once. Rotate replaces its old hash; revoke removes it. Credentials permit the client status and allocation APIs, never administration. Each identity has a 16-block/second refill rate and 32-block burst; requests contain 1–16 blocks. A block is 64 bytes. Pool availability can impose lower throughput. Rotation does not recall previously delivered data.

The server limits credential records, sessions, pending account attempts, administrative event history, HTTP connection count, body sizes and timeouts. Configuration writes are serialized, atomically replaced and synced. A process-held exclusive file lease prevents two servers from racing the same directory. Do not externally edit the live credential store.

## KagNode owner linking

Sign in locally, open Account and choose Connect or change KagNode Account. Re-enter the local key, then approve the displayed application and exact local origin at KagNode in the same tab. The server requests `profile.read email.read` with an unpredictable browser-bound nonce, state and S256 PKCE challenge. It accepts only the exact local `/auth/kagnode/callback` and validates the issuer in both the callback and fresh proof. It checks the first-exchange authorization's issuer, client, installation, purpose, nonce, immutable subject and time bounds, and checks the retrieved profile's matching identity.

Only that explicitly linked subject can use Login with KagNode. No public registration, email matching or remote administrator role is accepted. Display name, approved profile photo and explicitly consented verified contact email update on successful provider sign-in; they do not replace the local recovery credential. Server-to-server grants remain out of the browser and are revoked after retrieving the sign-in profile; this adapter does not persist refresh credentials or synchronize continuously in the background.

Disable integration or disconnect the owner in Account by confirming the local key. Pending attempts and existing admin sessions are invalidated; local sign-in still works. `KAGNODE_ACCOUNT_SYNC=off` disables provider use at the process level. Cancellation consumes the local attempt; an interrupted or ambiguous exchange requires starting again. Account linking never grants consumer API credentials.

## API boundaries

`POST /api/v1/admin/login` exchanges the local key for a 15-minute HttpOnly/SameSite=Strict cookie and a CSRF value. Loopback HTTP cookies intentionally have no Secure flag; these are not internet-facing cookies. Browser writes use `X-CSRF-Token`; client-key changes and account-policy changes additionally require the local key. `GET /api/v1/admin/status` returns aggregate engine health, client IDs and bounded sanitized current-process events, never credential hashes or raw outputs.

`POST /api/v1/admin/clients` supports create, rotate and revoke. `POST /api/v1/admin/logout` invalidates the session and pending owner sign-ins. Provider interfaces are under `/api/v1/admin/kagnode/`: availability, start, poll, cancel, settings and policy. PKCE verifiers, state and authorization codes remain in bounded server memory; requests expire after ten minutes and cannot survive restart. The issuer code and local callback deadline last at most 60 seconds. The shared Core controller opens the issuer in the same tab and resumes the local attempt after return; no device code is shown.

Administrator-key recovery requires trusted local access to `admin.key`; preserve it separately from public source. Do not delete the store to change client access. Loss or suspected disclosure of the local key requires taking this instance offline and a deliberate local credential recovery procedure; no unauthenticated web reset exists.

## Interrupted configuration writes

A failure before atomic replacement leaves the prior configuration active. If replacement occurred but directory durability could not be confirmed, the process blocks all new consumer authorizations and administrative sessions. It does not retry a mutation against stale in-memory permissions. Stop it, inspect the private configuration and retained backup, then deliberately restart after resolving storage health. Provider operations have a six-second total budget within the HTTP/browser limits; timeout or cancellation invalidates their local attempt. Deadlines are checked again before owner/session finalization.


The pending-attempt cookie is HttpOnly/SameSite=Lax for the cross-site top-level GET callback; the administrator session remains SameSite=Strict. Local key authorization is captured server-side before linking and bound to the owner generation, so a callback does not depend on forwarding the administrator cookie through the issuer redirect. The clean landing's same-origin API request finishes the existing attempt. Owner changes, policy changes, logout, restart, expiration and cancellation invalidate it.

Only GET `/auth/kagnode/callback?state=…&iss=…&code=…` (or `error=…`) and GET `/admin` or `/admin/link` with a 64-character opaque `kagnode_attempt` are query exceptions. Duplicate, unknown, bearer-token and wrong-issuer fields are rejected. The callback has no-store/no-referrer headers; the Core controller removes the resume ID from the visible URL. No access or refresh token is placed in a browser URL. An ambiguous code exchange is terminal and must be restarted, never retried as a sign-in proof.
