# Security

KagNode Entropy 0.1.0-alpha.1 is a source preview, not a production randomness or confidential-computing service. Do not use its experimental mode for passwords, cryptographic keys, gambling, regulated sampling or other applications that require assessed randomness.

For ordinary secret generation, use the operating system's cryptographic RNG directly. The default mode uses that RNG, but the surrounding custom composition still needs independent review.

Never include generated outputs, live API tokens, audit keys, raw user frames, memory dumps or private account data in a public issue. For a public, non-sensitive defect, open an issue with a minimal fictional reproduction. This repository does not yet offer a separate private reporting address or an SLA; use GitHub's private vulnerability reporting if it is enabled for the repository.

Review [threat model](docs/THREAT-MODEL.md), [audit/defense limits](docs/HARDENING.md) and [TEE roadmap](docs/TEE.md). Passing tests is not a security certification or proof of entropy, uniqueness, physical erasure or enclave isolation.
