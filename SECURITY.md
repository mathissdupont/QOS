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

## Experimental Status

QOS contains prototype and research-oriented subsystems. Experimental status does not make security
bugs unimportant, but it affects severity, support expectations, and release timelines.
