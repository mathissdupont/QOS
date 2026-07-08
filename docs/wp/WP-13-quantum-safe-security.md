# WP-13: Quantum-safe security — PQC, kernel crypto, secure channels

- Status: 🔴 not started
- Epic: E-54 (quantum-safe cryptography — new); adjacent to E-52 (users/permissions)
- ADRs: new ADR for the crypto policy (algorithm suite, hybrid rule, entropy model)
- Commits: (appended as delivered)

## Goal

QOS presents itself as a quantum operating system, so its own security must survive a quantum
adversary. Classical public-key cryptography (RSA, ECDH/ECDSA) is broken by Shor's algorithm on a
large fault-tolerant quantum computer, and **"harvest now, decrypt later"** makes that a *today*
problem for any long-lived secret sent over a classical channel — provider tokens, job payloads,
results. Symmetric ciphers and hashes survive with larger parameters (Grover halves brute-force
exponents), so the floor is 256-bit keys and SHA-256/SHA3-256 or wider.

**Policy: every cryptographic surface QOS grows is quantum-safe from day one.** NIST-standardized
algorithms only: **ML-KEM** (FIPS 203) for key establishment, **ML-DSA** (FIPS 204) and
**SLH-DSA** (FIPS 205) for signatures — hybridized with classical X25519/Ed25519 during the
transition, matching current IETF/industry practice for TLS hybrid key exchange. This lands
before the WP-12 provider channel carries a single real credential.

## Steps (planned slices)

- [ ] **Slice 1 — crypto policy ADR.** Algorithm suite and parameter sets (ML-KEM-768,
  ML-DSA-65 as defaults; SLH-DSA for long-lived release signing), the hybrid rule
  (classical + PQ combined KDF), and a license review of candidate `no_std` implementations
  (RustCrypto `ml-kem`/`sha2`/`sha3`/`hkdf`/`chacha20poly1305` — Apache-2.0/MIT, commercially
  safe). No from-scratch primitives: wrap audited crates.
- [ ] **Slice 2 — kernel entropy + CSPRNG.** RDSEED/RDRAND (CPUID-gated, universal — no
  machine-specific assumptions) with a timing-jitter fallback; SP 800-90B-style health tests
  (repetition + adaptive proportion); one `rand` API for kernel subsystems and, later, userland.
  Also replaces the ad-hoc PRNG used for quantum shot sampling.
- [ ] **Slice 3 — primitives with KATs.** SHA-256/SHA3-256, HMAC/HKDF, ChaCha20-Poly1305 (or
  AES-256-GCM); NIST known-answer tests run on the host **and** as a boot power-on self-test
  (one serial line: pass/fail per primitive).
- [ ] **Slice 4 — ML-KEM-768.** Encapsulation/decapsulation wired into the kernel crypto module;
  KATs on host + in QEMU.
- [ ] **Slice 5 — ML-DSA verify path.** Release/update images signed at build time; the kernel
  verifies signatures before applying anything (ties into the WP-11 install/update path).
- [ ] **Slice 6 — sealed secrets.** Provider tokens encrypted at rest (AEAD under a device
  secret), never written to serial/UI logs; a redaction audit over existing log sites.
- [ ] **Slice 7 — hybrid PQ channel.** X25519 + ML-KEM-768 handshake profile for the WP-12
  provider transport (over WP-10 TCP); proven against the mock provider first.
- [ ] **Slice 8 — crypto audit hook.** CI/grep check that no classical-only public-key usage
  ships; documented exceptions list (empty is the goal).

## Acceptance criteria

- NIST KAT vectors pass on the host and inside QEMU (serial evidence).
- Boot shows a crypto self-test line; a failed self-test is loud, not silent.
- No secret material ever appears in serial or UI logs (audited).
- The provider channel design is hybrid PQ before any real credential is used.
- `SECURITY.md` states the quantum-safe policy; ADR records the suite and rationale.

## Notes & gaps

- Constant-time discipline: rely on the crates' CT guarantees; no secret-dependent branches or
  indexing in our glue code.
- Slices 1–6 are host-testable and independent of networking; only slice 7 depends on WP-10.
- UEFI Secure Boot (shim/db signing) is the eventual root of trust — a separate later WP; this
  WP covers everything above the firmware.
- Professional cryptographic review is required before any claim of production security.
