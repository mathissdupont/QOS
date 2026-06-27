# Summary

<!-- What does this PR change, and why? -->

## Related issues

<!-- e.g. Fixes #123 -->

## How verified

<!-- For kernel changes: paste the QEMU serial output or attach a screenshot showing it working.
     For qos-core: note the tests you ran (cargo test -p qos-core --features std). -->

## Checklist

- [ ] Builds: `cargo os-build` and `cargo os-bootimage`
- [ ] Core tests pass: `cargo test -p qos-core --features std`
- [ ] Kernel code stays `no_std`
- [ ] Added/updated an ADR in `docs/adr/` if this changes an architectural decision
- [ ] Repository content is in English
