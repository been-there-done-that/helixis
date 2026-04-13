# RFC-003: Implementation Plan

## Status

Draft v1

## Objective

Define a realistic implementation sequence for Helixis that ships core value
early while preserving the architecture needed for production hardening.

## Delivery Strategy

Build the system in vertical slices, not layer silos.

Bad sequence:

- spend weeks building a perfect scheduler
- then build artifacts
- then add executors

Good sequence:

- submit one task
- lease one task
- run one task
- report one result
- then harden correctness, scale, and ergonomics

## Recommended Phases

### Phase 0: Foundation

Deliverables:

- convert repo to a Cargo workspace
- stand up Postgres and S3-compatible storage locally
- add config loading, tracing, and migration framework
- define core domain types and task state machine
- write the first architecture decision records
- define the minimum sandbox and runtime isolation contract for untrusted code
- define payload, result, and log size limits
- define global content-addressed artifact identity rules

Success criteria:

- `cargo test` and local infra boot reliably
- schema migrations run cleanly
- shared types compile independently from transport and storage

### Phase 1: Minimal End-To-End Task Execution

Scope:

- public task submission API
- artifact metadata registration
- executor registration
- long-poll task assignment
- single executor runtime pack for Python
- completion reporting
- platform-managed secrets injection
- persisted logs plus basic live log streaming

Implementation notes:

- keep scheduling logic inside the control-plane service
- use Postgres leasing with `SKIP LOCKED`
- inline small payloads and store large payloads in object storage
- accept only externally built and packaged artifacts

Success criteria:

- user can submit a task and get a result
- executor restarts do not corrupt task state
- duplicate submissions respect idempotency keys
- the runtime path never performs user build steps
- running tasks can be inspected via live logs

### Phase 2: Reliability And Retry Semantics

Scope:

- attempt table and execution history
- retry policy with exponential backoff and jitter
- lease expiry reconciliation
- task timeout enforcement
- dead-letter handling
- baseline egress and sandbox restrictions for untrusted workloads
- cancellation flow from control plane to executor
- log truncation and bounded retention behavior

Success criteria:

- crashed executor attempts are recovered
- timed out tasks transition correctly
- duplicate completion is safe
- cancellation reliably terminates local subprocesses

This phase matters more than adding new languages. Reliability is the real
product.

### Phase 3: Multi-Runtime Support

Scope:

- Node runtime pack
- runtime registry abstraction
- artifact cache manager
- per-runtime executor capability advertisement

Success criteria:

- one control plane can schedule to multiple runtime pools
- runtime pack mismatches are rejected before execution
- cache hit ratio is visible in metrics

### Phase 4: Scheduling Quality And Fairness

Scope:

- scoring model for locality, load, and fairness
- tenant quotas
- priority support
- rate-limit support

Success criteria:

- noisy tenants cannot starve others
- backlog distribution remains understandable and observable

### Phase 5: Autoscaling

Scope:

- backlog and throughput metrics
- scaling planner
- local and one cloud adapter implementation
- cooldown and min/max controls

Success criteria:

- executor pools react to demand without oscillating wildly
- scale decisions are explainable from logs and metrics
- local and ECS/Fargate adapters both work for the same runtime pool model

### Phase 6: Production Hardening

Scope:

- authN/authZ hardening
- secret injection
- structured audit logging
- resource isolation improvements
- load testing
- runbooks and dashboards

Success criteria:

- the system can survive expected failure modes with operator visibility
- SLOs are measurable

## Detailed Workstreams

### Workstream A: Domain And Persistence

Key tasks:

- define task, attempt, executor, lease, artifact, and schedule schemas
- implement repository interfaces
- add migrations and query benchmarks

Primary risk:

- getting transaction boundaries wrong

### Workstream B: Public API

Key tasks:

- create task submission and status endpoints
- implement request validation and error contracts
- support idempotency keys and tenant scoping

Primary risk:

- accidental mismatch between external API and internal task semantics

### Workstream C: Executor Agent

Key tasks:

- registration, poll, heartbeat, completion clients
- slot management
- runtime pack loading
- subprocess supervision

Primary risk:

- local execution lifecycle bugs around timeouts and cleanup

### Workstream D: Artifact Management

Key tasks:

- artifact metadata API
- object storage upload and fetch
- local executor cache
- checksum enforcement

Primary risk:

- conflating artifact registration with build concerns

### Workstream E: Operations

Key tasks:

- metrics
- tracing
- dashboards
- alerting
- load and chaos tests

Primary risk:

- no visibility into lease loss or retry storms

## Suggested v1 Technology Choices

- HTTP: `axum`
- async runtime: `tokio`
- database: Postgres
- SQL layer: `sqlx`
- object storage: S3-compatible API
- local infra: Docker Compose
- telemetry: `tracing` plus OpenTelemetry
- auth: API keys or signed service tokens initially

## Suggested Milestone Order

1. Workspace conversion and migrations
2. Task domain model and Postgres schema
3. Submit-task API
4. Executor registration and poll
5. Python runtime execution
6. Completion and status retrieval
7. Retry and lease reconciliation
8. Node runtime support
9. Quotas and fairness
10. Autoscaling

## Suggested Folder Ownership As The Repo Grows

- `apps/control-plane`
  HTTP handlers, service startup, wiring only
- `apps/executor`
  executor bootstrap and local process management wiring
- `crates/helixis-domain`
  canonical task lifecycle and invariants
- `crates/helixis-application`
  use cases and orchestration
- `crates/helixis-persistence`
  all SQL and DB mappings
- `crates/helixis-runtime-*`
  language-specific execution

This avoids a future where every service imports every other service directly.

## Operational Milestones

Before first shared environment:

- health checks
- migration rollback plan
- task and attempt dashboards
- audit logs for task submission and artifact publication
- documented size limits for payloads, results, and logs
- live log viewing for active attempts

Before first external tenant:

- quota enforcement
- secret handling policy
- log retention and redaction rules
- incident runbooks
- explicit sandbox and egress policy for untrusted code

Before hard multi-tenant production:

- stronger sandboxing
- abuse protection
- egress control
- resource accounting per tenant

## What To Avoid Early

- building a workflow engine before the task engine is reliable
- introducing Kafka, NATS, and Redis all at once
- hiding scheduler logic behind too many abstractions before it stabilizes
- optimizing for multi-region before a single-region design is proven

## Recommended First Concrete Implementation Target

Target a demo that proves:

- one Python artifact
- one executor
- one control-plane binary
- one Postgres database
- one object store

and exercise the full loop:

```text
submit -> queue -> poll -> lease -> execute -> complete -> fetch result
```

If that loop is clean, the rest of the system can be layered on with confidence.
