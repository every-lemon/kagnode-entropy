# Security

KagNode Entropy 5 distributes unchanged bytes from the host operating system's cryptographic RNG. Its stable scope is an authenticated, bounded, loopback service on a trusted Linux host. Using the OS RNG directly in a local application avoids this additional service boundary.

The supplemental video/timing mixer is explicitly experimental and must not supply production keys, passwords, regulated sampling or other applications requiring assessed randomness. No min-entropy assessment or cryptographic certification is claimed. Enclave profiles are research-only; they do not provide runtime attestation or secret release.

Admin sessions are separate from consumer credentials. Protect the private administrator directory, its local recovery key and backups. KagNode Account sign-in requires explicit owner linking and a fresh browser-bound approval; email and role claims cannot create an administrator. Local sign-in remains independent.

Do not include generated output, credentials, audit keys, raw frames, memory dumps or user data in public reports. Report non-sensitive defects with fictional reproductions. Use GitHub private vulnerability reporting when enabled; no private response SLA is promised.

Read [threat model](docs/THREAT-MODEL.md), [administration](docs/ADMINISTRATION.md) and [audit/defense limits](docs/HARDENING.md). Ordinary processes cannot defeat a compromised kernel, privileged debugger or hostile host. Best-effort zeroization does not guarantee physical erasure of every copy.
