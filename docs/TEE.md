# Confidential-computing roadmap — not implemented hardware

The released runtime is an ordinary Linux process. No enclave is launched, no quote is generated or verified, and no production client trusts hardware evidence. `tee::Policy` validates design-time measurement pins; `inspect_mock` consumes synthetic fields and **always** returns `hardware_verified=false` and `may_release_secrets=false`, even when the fixture matches. It is not an attestation verifier.

## Distinct proposed profiles

| Area | Intel SGX | AWS Nitro Enclaves |
| --- | --- | --- |
| Boundary to design for | Selected application code/data in an SGX enclave | An isolated enclave VM using Nitro Hypervisor isolation |
| Evidence to implement | SGX quote verification, collateral, TCB status and freshness policy | Nitro COSE document/signature and AWS certificate-chain verification |
| Example pins | MRENCLAVE, MRSIGNER, product ID, minimum ISV SVN; reject debug | PCR0 image, PCR1 kernel/bootstrap, PCR2 application; reject debug/zero measurements; add parent/signer constraints where needed |
| I/O | Explicit untrusted host calls/buffers and enclave RNG integration | Constrained vsock protocol, enclave RNG and untrusted parent transport |
| Current project status | Policy fixtures and documentation only | Policy fixtures and documentation only |

SGX attestation identifies an instantiated enclave under a platform TCB policy. It does not continuously attest every host-kernel change or cause automatic self-termination when root changes the OS. Platform updates and accepted TCB states require review. [Intel SGX attestation details](https://www.intel.com/content/www/us/en/security-center/technical-details/sgx-attestation-technical-details.html)

Nitro evidence is signed by the Nitro infrastructure and contains measured configuration. Its PCRs refer to distinct artifacts; debug-mode evidence has zero PCR values. SGX PRM/MEE terminology must not be applied as though it describes every Nitro platform. [AWS cryptographic attestation](https://docs.aws.amazon.com/enclaves/latest/user/set-up-attestation.html)

## Required client protocol before any real secret release

1. Define an exact target, dependencies, compiler, build recipe and trusted configuration. Reproduce the build and pin reviewed artifact measurements; a source repository name is not a hardware measurement.
2. Generate a fresh, unpredictable client challenge; track it as single-use in a bounded client/session store. A time window alone does not stop replay.
3. Bind an enclave-generated channel key and protocol context to the signed evidence. Verify vendor roots, chain, signature, evidence structure, revocation/collateral, platform TCB, debug policy and exact measurement allowlist.
4. Verify the client's nonce, bounded age using a trusted verifier clock, rollback/version policy and channel-key binding. Release secrets only through the verified encrypted channel after all checks succeed.
5. Re-attest on restart or policy transition, maintain a measured update/revocation workflow and test negative cases on actual selected hardware.

The optional Nitro nonce/public-key fields enable protocol binding only when a verifier checks them; merely receiving these fields is insufficient. [AWS root-of-trust verification](https://docs.aws.amazon.com/enclaves/latest/user/verify-root.html)

Current fixture tests cover measurement/profile mismatch, debug rejection, minimum SVN, challenge/key mismatch and stale/future timestamps. The mocks neither verify signatures nor consume single-use challenges, and cannot be used to authorize output.

## Residual risks and unsupported claims

The host can deny service, schedule execution, affect I/O, replay unprotected state, observe metadata or exploit application vulnerabilities. Side channels, dependency compromise and platform/vendor vulnerabilities remain part of the threat model. Snapshot and rollback behavior must be tested for the selected platform; there is no universal promise that reads return `0xFF`.

Memory encryption coverage differs by processor and platform. The project has not validated cold-boot resistance, physical probing, cache coverage, encrypted memory paths or power-loss behavior. An interrupted process cannot run guaranteed cleanup. No “1000% security”, zero-vulnerability, kernel-rootkit immunity, absolute historical secrecy or certified erasure claim is made.
