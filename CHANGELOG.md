# Revision history

## 0.1.0-alpha.2

Added for 0.1.0-alpha.2.

Opt-in bounded direct-video ingestion, source status/failure handling and controlled decoder cleanup. Fixed late responses restoring cleared output, stale client context/attribution after disconnect, non-JSON error feedback and invalid source URLs creating an unnecessary audit session. Added product branding and regression checks. Real decoder/stream and enclave qualification remain unverified.

## 0.1.0-alpha.1

Added for 0.1.0-alpha.1.

Initial Rust source preview: two mixing modes, bounded authenticated allocation, locally served KagNode interface, optional metadata audit and tracer policy, explicit threat model, and simulation-only SGX/Nitro policy fixtures. No production entropy service or enclave target is released.
