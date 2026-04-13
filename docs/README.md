# Helixis Documentation

This folder contains the initial architecture and implementation RFC set for
Helixis, a pull-based distributed execution platform for polyglot workloads.

Current decisions used in this draft:

- `Helixis` is the canonical platform and codebase name.
- The backend is implemented entirely in Rust.
- v1 is focused on async task execution only.
- workloads are untrusted user-uploaded code.
- artifacts are built externally and uploaded as final packaged outputs.
- first deployment targets are local and ECS/Fargate.
- execution semantics are at-least-once.
- v1 is operationally single-tenant.
- outbound internet access is enabled by default for task sandboxes.
- priorities, rate limits, and cancellation are part of the task model.
- payloads, results, and logs are size-limited by platform policy.
- artifact versions use global content-addressed identifiers.
- v1 uses best-effort fair scheduling with hard configured caps.
- single-region deployment is the initial operating model.
- observability defaults to OpenTelemetry plus Prometheus and Grafana.
- live log streaming and retained post-run logs are both supported.

Documents:

- [rfc-001-system-architecture.md](./rfc-001-system-architecture.md)
  End-to-end platform architecture, control plane and execution plane design.
- [rfc-002-rust-codebase.md](./rfc-002-rust-codebase.md)
  Proposed Rust workspace layout, crate boundaries, service interfaces, and
  implementation conventions.
- [rfc-003-implementation-plan.md](./rfc-003-implementation-plan.md)
  Delivery phases, operational milestones, and recommended implementation
  order.
- [rfc-004-open-questions.md](./rfc-004-open-questions.md)
  Decisions that still need product or infrastructure clarification.

Recommended reading order:

1. System architecture
2. Rust codebase architecture
3. Implementation plan
4. Open questions
