# ADR-0001: Adopt the ADR process

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** —

## Context

Until now, architectural decisions in the QOS repository lived in free-form documents
such as `ROADMAP.md` and `docs/VISION.md`. Concrete problems were observed with this
approach:

- The two documents contradicted each other (e.g. networking was "DONE" in one and
  "absent" in the other).
- Unfinished work (a stub TLS implementation, a TODO remote backend) was marked as
  "complete" at the document level, creating a trust-eroding gap between docs and code.
- The "why did we do it this way?" knowledge was not preserved anywhere.

The project is now committing to a clear goal (see ADR-0002) and will make many
interdependent architectural decisions on the way there. Those decisions must be
traceable.

## Decision

We will record all significant architectural decisions as numbered, immutable ADR files
under `docs/adr/`. The format and rules are fixed by [`README.md`](README.md) and
[`template.md`](template.md).

- ROADMAP/VISION are stripped of marketing language; the "actual state" lives in a single
  status document, while the architectural "why" moves into ADRs.
- A feature counts as "done" only when it is verifiable in code.

## Rationale

ADRs are lightweight (markdown, in-repo), reviewable (via PR), and durable. They preserve
decision context to answer "why was it built this way" later, and they structurally
prevent the doc-vs-code contradiction.

## Consequences

### Positive

- Decision rationale is permanent and searchable.
- New contributors can understand the project by reading the ADRs in order.
- The doc-vs-code contradiction shrinks (rule: no "done" without code verification).

### Negative / Trade-offs

- A small writing overhead on every significant decision.

### Neutral / Follow-ups

- Aligning `ROADMAP.md` and `VISION.md` with the actual state is tracked as separate work.
