# ADR-0011: Quantum cloud connectivity via a host TLS proxy

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0004 (remote backends are QHAL impls), ADR-0002 (cloud is in scope)

## Context

A core goal (ADR-0002) is connecting to quantum cloud providers (IBM, IonQ, Google, AWS
Braket, Azure Quantum, …). Every one of these APIs requires **HTTPS/TLS**. Verified facts
about the current code:

- `qos-os-kernel/src/http.rs` contains a from-scratch in-kernel TLS implementation that is a
  **non-functional skeleton**: the ClientHello omits a real `key_share` (sends a truncated
  extension), no ECDH key exchange or key schedule is performed, no AEAD encryption, no
  certificate verification, and the handshake code comments say it "assumes handshake
  completes." Its randomness is derived from timer ticks (not cryptographically secure).
- The `RemoteQpuBackend` `job_status`/`get_result`/`cancel` are `TODO` stubs returning
  `None`/`false`, and no local→remote job-id mapping is kept.

Writing correct, secure TLS 1.3 from scratch in a kernel is a large effort and a serious
security liability (custom crypto).

## Decision

**Do not implement TLS inside the bare-metal kernel.** Quantum cloud access goes through the
**host daemon (`qosd`) acting as a TLS-terminating proxy** that uses a vetted Rust TLS stack
(**rustls**):

- In the **host embodiment**, `qosd` connects directly to provider APIs over rustls.
- In the **bare-metal embodiment**, the kernel speaks a plain, local protocol to `qosd`
  (which runs on trusted, co-located infrastructure), and `qosd` performs the TLS leg to the
  provider. The kernel never terminates TLS.
- The QHAL `RemoteQpuBackend` (ADR-0004) targets the proxy endpoint; provider-specific request
  building, polling, result parsing, and the local→remote id mapping are implemented there
  (fixing the current stubs).
- The in-kernel TLS code is **feature-gated off and removed from the default build**, and all
  "TLS supported" claims in docs are withdrawn until this lands.

## Rationale

- Reusing rustls eliminates a security-critical custom-crypto effort and is correct by
  construction.
- `qosd` already exists as the host daemon; making it the network egress point fits the
  layered architecture (ADR-0003) and keeps the kernel small.
- A clear trust boundary (kernel ↔ trusted local proxy) is honest about where TLS happens.

## Consequences

### Positive

- Real cloud QPU connectivity becomes achievable and secure, soon, without kernel crypto.
- The kernel shrinks; one less liability.

### Negative / Trade-offs

- The bare-metal embodiment depends on a co-located `qosd` for cloud access; it cannot reach
  the internet fully standalone. Accepted: matches real control-plane deployments.
- `qosd` must be re-enabled in the workspace (it was disabled) and gain the proxy/backends.

### Neutral / Follow-ups

- Authentication/secrets (provider API keys) live in `qosd`, never in the kernel image.
- A future fully-standalone kernel TLS (via a vetted `no_std` TLS crate) could revisit this,
  but is explicitly not the plan now.
