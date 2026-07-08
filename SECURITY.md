# Security Policy

QOS is an experimental bare-metal operating system project. Security reports are welcome, but the
project should not be treated as production-ready software.

## Reporting a Vulnerability

For security-sensitive reports, follow the organization security policy:

https://github.com/Heptapus-Open-Code-Organization/.github/blob/main/SECURITY.md

Please do not publish exploit details, credentials, private logs, or hardware-sensitive details in
a public issue before maintainers have had a reasonable chance to investigate.

## What To Include

- affected commit or branch;
- how QOS was run;
- QEMU/hardware details;
- steps to reproduce;
- expected impact;
- serial logs or screenshots when safe to share.

## Scope

Relevant areas include:

- kernel memory safety and isolation bugs;
- syscall/user-pointer validation issues;
- driver packet/device parsing bugs;
- boot, ACPI, PCI, USB, storage, or networking flaws;
- credential/token handling in future cloud QPU paths;
- build/release-chain issues.

## Quantum-Safe Direction

QOS adopts a quantum-safe cryptography policy: cryptographic surfaces (cloud QPU provider
channels, update/release signing, secrets at rest) are designed around NIST post-quantum
standards — ML-KEM (FIPS 203), ML-DSA (FIPS 204), SLH-DSA (FIPS 205) — hybridized with
classical algorithms during the transition. The plan, algorithm suite, and status live in
`docs/wp/WP-13-quantum-safe-security.md`. Reports about weaknesses in this design (algorithm
choice, entropy handling, secret storage, downgrade paths) are explicitly in scope.

No production-security claim is made until the implementation has had professional
cryptographic review.

## Experimental Status

QOS contains prototype and research-oriented subsystems. Experimental status does not make security
bugs unimportant, but it affects severity, support expectations, and release timelines.
