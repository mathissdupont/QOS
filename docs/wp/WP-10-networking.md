# WP-10: Networking — a NIC that exists + a usable TCP/IP path

- Status: 🔴 not started
- Epic: E-50 (networking), feeds E-81 (cloud QPU access)
- ADRs: ADR-0011 (cloud proxy concept); new ADR for the driver choice
- Commits: (appended as delivered)

## Goal

QOS currently has an E1000 driver but q35 exposes an **e1000e (8086:10d3)** — so no NIC attaches
and the net stack idles (`[NET] E1000 init skipped`). TLS is a stub (gap G-07). A real OS needs
working egress; the quantum plane needs it for cloud QPUs (E-81).

## Steps (planned slices)

- [ ] **Slice 1 — a working NIC on q35.** Either extend the driver to e1000e (10d3) or switch
  QEMU to virtio-net and write the (smaller, modern) virtio-net driver — decide by ADR
  (virtio-net is the modern/universal-VM choice; e1000e serves real Intel NICs).
- [ ] **Slice 2 — DHCP + ICMP.** Lease an address in QEMU user networking; `ping` shell command
  round-trips; Devices/Monitor show link + IP.
- [ ] **Slice 3 — TCP + HTTP client.** Bring the existing `net`/`http` modules to a verified
  GET against a local test server; Terminal `fetch <url>`.
- [ ] **Slice 4 — TLS decision.** Real TLS in-kernel is a large dependency; decide (ADR) between
  a vetted no_std TLS library or the ADR-0011 local proxy for cloud-QPU calls.

## Acceptance criteria

Per slice: verified packet exchange in QEMU (serial logs + UI state), 0-fault boot, ADR + WP
updated. End state: an HTTP fetch from inside QOS with the NIC visible in Devices.

## Notes & gaps

- Security: incoming-packet parsing must be bounds-checked like all untrusted input (ADR-0020
  principle); no dynamic allocation sized by packet contents without caps.
