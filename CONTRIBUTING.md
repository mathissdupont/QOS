# Contributing to QOS

Thanks for your interest in QOS! This document explains how to build, test, and submit changes.
All repository content (code, comments, docs) is in **English**.

## Getting set up

1. Install the Rust toolchain. The pinned nightly is declared in `rust-toolchain.toml` and is
   selected automatically by `rustup` — it includes `rust-src`, `llvm-tools-preview`, and the
   `x86_64-unknown-none` target.
2. Install the boot-image tool: `cargo install bootimage`.
3. Install [QEMU](https://www.qemu.org/) (`qemu-system-x86_64`) to run the kernel.

A Docker-based build is available (`Dockerfile` / `docker-compose.yml`) if you prefer a clean
Linux toolchain.

## Build, run, test

```sh
cargo test -p qos-core --features std   # portable core unit tests (host)
cargo os-build                          # build the bare-metal kernel
cargo os-bootimage                      # build a bootable image
cargo os-verify                         # headless boot + Ring-3 quantum demo smoke test
```

On Windows, `./run-qos.ps1 -Build` builds and launches QEMU. See the [README](README.md) for the
in-OS demo commands (`threadtest`, `proctest`, `faulttest`, `gdesk`, …).

## Project layout

- `crates/qos-os-kernel` — the kernel (package `os`).
- `crates/qos-core` — the portable control-plane core (`no_std + alloc`, optional `std`).
- `crates/qos-abi` — shared ABI types.
- `docs/adr/` — Architecture Decision Records. **Read these before large changes**, and add a new
  ADR when you make a decision with long-term consequences.
- `docs/PLAN.md` — the phased roadmap.

## Making changes

- Keep the kernel `no_std`. Avoid pulling in crates that require `std` into kernel code.
- Match the surrounding style: comment density, naming, and idioms of nearby code.
- Prefer small, focused PRs. Describe **what** changed and **why**, and how you verified it.
- For anything touching boot, paging, interrupts, or the scheduler, include the QEMU output (or a
  screenshot) showing it working — these areas are easy to break silently.
- Update or add an ADR when your change alters an architectural decision.

## Commit & PR conventions

- Write clear commit messages (imperative mood: "add", "fix", "refactor").
- Open a PR against `main`. Fill in the PR template. CI must pass.
- Reference any related issue (`Fixes #123`).

## Reporting bugs / requesting features

Use the issue templates. For bugs, include how you ran QOS (QEMU command / `run-qos.ps1`), the
toolchain version, and the serial log if available.

## License of contributions

QOS is licensed under the **GNU Affero General Public License v3.0** (AGPL-3.0-only; see
[LICENSE](LICENSE)) — an OSI-approved, strong-copyleft open-source license. By submitting a
contribution, you agree that it is licensed under the same terms and that you have the right to
license it that way. No separate contributor license agreement (CLA) is required. If you have
licensing concerns, please open an issue first.
