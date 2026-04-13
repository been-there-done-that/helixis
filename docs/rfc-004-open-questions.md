# RFC-004: Clarifications And Remaining Open Questions

## Purpose

This document tracks decisions already made and the questions that still affect
implementation.

## Resolved Decisions

The following items are now fixed for v1:

1. `Helixis` is the canonical platform and codebase name.
2. v1 is async task execution only. Scheduled jobs and workflow orchestration
   come later.
3. workloads are untrusted user-uploaded code, similar to Lambda-style
   execution
4. the platform accepts final build or packaged output only
5. local and ECS/Fargate are the first deployment targets
6. execution semantics are at-least-once
7. v1 is operationally single-tenant
8. outbound internet access is enabled by default for task sandboxes
9. priorities, rate limits, and cancellation are in scope for v1
10. payloads, results, and logs are bounded by platform limits
11. recommended secrets model is platform-managed encrypted secrets injected at
    execution start, with room to add short-lived provider credentials later
12. v1 supports language runtimes only, not generic container-image execution
13. Rust and Go artifacts are uploaded as binaries only
14. artifact versions use global content-addressed identifiers with ownership
    stored separately
15. scheduling is best-effort fair with hard configured concurrency and rate
    limits
16. v1 is single-region
17. observability defaults to OpenTelemetry plus Prometheus and Grafana
18. live log streaming and retained post-run logs are both supported
19. results and logs are retained indefinitely for now
20. suggested v1 size limits are 1 MiB payload, 10 MiB result, and 50 MiB
    retained logs per attempt
21. sync request-response execution may be added later for short-lived workloads,
    but v1 remains async-first
22. the suggested default v1 size limits are accepted

## Remaining Questions

There are no blocking product or architecture questions left for the v1 RFC
set. Any further decisions can now be captured as implementation ADRs rather
than foundational architecture clarifications.
