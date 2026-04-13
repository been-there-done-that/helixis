# RFC-002: Rust Codebase Architecture

## Status

Draft v1

## Objective

Define a Rust-first codebase structure that supports:

- multiple deployable services
- shared domain contracts
- clear ownership boundaries
- testability
- future extraction of components without a rewrite
- an execution plane designed for untrusted user workloads

## Guiding Principles

- keep business rules in libraries, not binaries
- prefer explicit module boundaries over a giant `src/` tree
- isolate infrastructure code behind traits and adapters
- make executor and control-plane code compile independently
- keep transport DTOs separate from domain models where it matters

## Recommended Repository Shape

Use a Cargo workspace rather than a single binary crate.

```text
helixis/
├── Cargo.toml
├── docs/
├── crates/
│   ├── helixis-api/
│   ├── helixis-scheduler/
│   ├── helixis-autoscaler/
│   ├── helixis-executor/
│   ├── helixis-artifact/
│   ├── helixis-domain/
│   ├── helixis-application/
│   ├── helixis-persistence/
│   ├── helixis-protocol/
│   ├── helixis-runtime/
│   ├── helixis-runtime-python/
│   ├── helixis-runtime-node/
│   ├── helixis-config/
│   ├── helixis-observability/
│   ├── helixis-auth/
│   ├── helixis-queue/
│   └── helixis-testkit/
├── apps/
│   ├── control-plane/
│   ├── executor/
│   └── admin/
├── migrations/
└── xtask/
```

This does not need to exist on day one, but the architecture should aim there.

## Workspace Strategy

### Binaries

Recommended binaries:

- `control-plane`
  Hosts HTTP API and optionally internal admin endpoints.
- `scheduler`
  Optional if scheduling is separated from API request handling.
- `autoscaler`
  Periodic reconciliation process.
- `executor`
  Runtime worker agent.

For a lean v1, `control-plane` may embed scheduler logic while still using
shared library crates so separation can happen later.

### Library Crates

#### `helixis-domain`

Pure domain types and rules:

- task state machine
- lease invariants
- artifact metadata
- runtime pack definitions
- quota and scheduling concepts
- priority and rate-limit policy
- cancellation semantics

Should avoid direct dependencies on HTTP, SQL, or cloud SDKs.

#### `helixis-application`

Application services and use cases:

- submit task
- assign task
- complete attempt
- heartbeat handling
- retry planning
- schedule reconciliation
- request cancellation
- enforce rate limits

This layer orchestrates domain objects and repository traits.

#### `helixis-persistence`

Database adapters and repositories:

- Postgres repositories via `sqlx`
- transaction helpers
- query models and row mapping
- migration bootstrapping

This crate owns SQL, indexes, and transaction boundaries.

#### `helixis-protocol`

Shared protocol contracts:

- HTTP request/response DTOs
- executor poll payloads
- API error envelope
- versioned wire schemas

This keeps API and executor in sync without importing transport stacks into
business logic.

#### `helixis-runtime`

Common runtime execution interfaces:

- `RuntimeAdapter`
- `Sandbox`
- `ArtifactFetcher`
- `ExecutionSupervisor`

Runtime-specific crates implement these interfaces.
This layer should be written assuming user code is hostile.

#### `helixis-runtime-python` and `helixis-runtime-node`

Language-specific execution behavior:

- artifact materialization
- command construction
- environment shaping
- language-specific health checks

These crates should not know about API routes or scheduler internals.

#### `helixis-queue`

Queueing abstractions and selection logic:

- task candidate queries
- leasing policy
- fairness scoring
- backoff eligibility
- priority ordering
- rate-limit admission checks

This is useful even if v1 uses Postgres directly because queue semantics are a
separate concern from general persistence.

#### `helixis-artifact`

Artifact lifecycle logic:

- checksum validation
- object storage adapters
- cache key generation
- signed URL or stream helpers

#### `helixis-auth`

Authentication and authorization primitives:

- API token validation
- executor registration credentials
- tenant scoping helpers
- secret access policy helpers

#### `helixis-config`

Typed configuration loading:

- environment variables
- config files
- per-service settings structs

#### `helixis-observability`

Common tracing, metrics, and logging bootstrap:

- tracing subscriber setup
- OpenTelemetry wiring
- metric naming helpers

#### `helixis-testkit`

Shared testing tools:

- fixture builders
- fake repositories
- integration harness utilities
- local executor simulation helpers

## Recommended Architectural Style

Use a layered architecture with explicit ports and adapters.

```text
Transport -> Application -> Domain
                |
                v
          Ports / Traits
                |
                v
          Adapters / Infra
```

This is not academic purity. It prevents the common failure mode where handler
functions, SQL, scheduling policy, and runtime spawning all become tangled in
the same modules.

## Suggested Module Breakdown

### `helixis-domain`

Possible modules:

- `task`
- `attempt`
- `lease`
- `artifact`
- `runtime_pack`
- `executor`
- `quota`
- `schedule`
- `errors`

Example concepts:

- `TaskStatus`
- `LeaseState`
- `RetryPolicy`
- `RuntimeConstraint`
- `ExecutorCapabilities`

### `helixis-application`

Possible modules:

- `submit_task`
- `poll_for_task`
- `start_attempt`
- `complete_attempt`
- `heartbeat`
- `requeue_expired_leases`
- `reconcile_schedules`
- `scale_runtime_pool`

Each use case should expose a focused service or command handler rather than a
single god-service.

### `helixis-persistence`

Possible modules:

- `repositories/tasks`
- `repositories/executors`
- `repositories/artifacts`
- `repositories/leases`
- `repositories/schedules`
- `repositories/quotas`
- `tx`

Prefer explicit repository traits in the application layer and concrete
`Postgres*Repository` implementations here.

## Trait Boundaries

Core traits likely needed:

```rust
pub trait TaskRepository {}
pub trait LeaseRepository {}
pub trait ExecutorRepository {}
pub trait ArtifactRepository {}
pub trait TaskQueue {}
pub trait ArtifactStore {}
pub trait RuntimeAdapter {}
pub trait ScalingAdapter {}
pub trait SecretProvider {}
pub trait RateLimiter {}
pub trait Clock {}
pub trait IdGenerator {}
```

Not every trait needs to exist immediately. Introduce them when a boundary is
actually useful for testing or future replacement.

## API Stack Recommendation

Recommended stack:

- `tokio` for async runtime
- `axum` for HTTP APIs
- `tower` and `tower-http` for middleware
- `serde` for serialization
- `sqlx` for Postgres
- `tracing` for structured logs
- `opentelemetry` for tracing export
- `uuid` and `time`
- `thiserror` and optionally `anyhow` at binary edges

Why not `diesel` for this project:

- `sqlx` generally works well for async service code
- explicit SQL is useful for queueing and lease queries
- scheduler-critical queries should be easy to inspect and tune

## Transport Design

### Public API

Endpoints for v1:

- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/executors/register`
- `POST /v1/executors/poll`
- `POST /v1/executors/heartbeat`
- `POST /v1/executors/complete`

Possible future endpoints:

- `POST /v1/tasks/{task_id}/cancel`
- `GET /v1/artifacts/{artifact_id}`
- `POST /v1/schedules`
- `GET /v1/tasks/{task_id}/logs/stream`

### Versioning

Version the external API early. Internal Rust modules can evolve faster than the
wire protocol.

### DTO Separation

Keep wire DTOs in `helixis-protocol`. Convert them into application commands in
handlers. This avoids leaking transport concerns everywhere.

## Database Strategy

### Postgres

Use Postgres as the authoritative store for:

- tasks
- attempts
- leases
- executors
- schedules
- quotas
- secrets
- rate-limit counters or policies
- active log stream session metadata

Use explicit migrations committed in the repo.

### Schema Design Notes

- use UUID primary keys
- use enums sparingly; strings plus validation can be easier to evolve
- partition very large attempt or log tables later, not prematurely
- keep payload and result blobs out of hot queue tables when large

### Transactions

Critical transaction boundaries:

- task submission plus idempotency check
- poll assignment plus lease creation
- completion plus terminal state update
- lease expiry plus task requeue

These are the heart of correctness.

## Executor Design In Rust

The executor should be its own binary and internal subsystem, not a thin script.

Suggested internal components:

- `RegistrationClient`
- `PollClient`
- `HeartbeatClient`
- `CapacityManager`
- `ArtifactCache`
- `TaskRunner`
- `RuntimeRegistry`
- `AttemptSupervisor`
- `ResultUploader`
- `CancellationWatcher`

### Concurrency Model

Use:

- one async control loop for polling and heartbeats
- bounded worker slots using semaphores
- per-attempt async supervision
- blocking runtime execution isolated with subprocess management

Avoid forcing the actual user code execution onto async tasks if it is really a
subprocess problem.

### Local State

Executor-local state should include:

- active attempts map
- cache inventory
- runtime pack registry
- current slot usage
- recent heartbeat state

Keep local state recoverable. The executor should tolerate process restarts
without requiring manual reconciliation.

The executor must also assume artifacts and payloads are adversarial and enforce
resource controls accordingly.

## Runtime Adapter Shape

Example conceptual interface:

```rust
pub trait RuntimeAdapter: Send + Sync {
    fn runtime_pack(&self) -> &RuntimePackId;

    async fn prepare(
        &self,
        artifact: &ResolvedArtifact,
        workdir: &std::path::Path,
    ) -> Result<PreparedExecution, RuntimeError>;

    async fn spawn(
        &self,
        prepared: PreparedExecution,
        limits: ResourceLimits,
    ) -> Result<RunningExecution, RuntimeError>;
}
```

The actual interface may evolve, but the separation should remain:

- artifact preparation
- process or sandbox creation
- execution supervision

## Configuration Model

Use typed config structs with per-service sections.

Example areas:

- database
- object storage
- auth
- API bind address
- scheduler timings
- lease durations
- executor concurrency
- payload, result, and log size limits
- default egress policy
- cache limits
- metrics export
- scaling adapters
- secret encryption settings
- region identity

Avoid unstructured env-var sprawl.

## Testing Strategy

### Unit Tests

Focus on:

- task state machine invariants
- retry calculations
- scheduler scoring
- quota rules
- protocol validation

### Integration Tests

Use real Postgres in tests for:

- task submission idempotency
- queue leasing correctness
- lease expiry and reassignment
- completion races

### End-To-End Tests

Provide a local harness that starts:

- Postgres
- MinIO
- control plane
- one executor

Then submit sample Python and Node artifacts and verify full lifecycle behavior.

### Failure Injection

Add scenarios for:

- executor crash mid-task
- heartbeat loss
- artifact download failure
- task timeout
- duplicate completion
- cancellation during execution
- log truncation under heavy output

This area is critical. Distributed systems usually fail in completion and retry
edges, not the happy path.

## Operational Conventions

Recommended repo conventions:

- `xtask` for local automation and repeatable developer workflows
- `justfile` optional for command ergonomics
- `clippy`, `rustfmt`, and `cargo deny` in CI
- one integration test package for queueing semantics
- architecture decision records for major changes

## Suggested Initial Migration Path

Starting from the current single-binary crate:

1. convert root `Cargo.toml` into a workspace
2. create `apps/control-plane` and `apps/executor`
3. create `crates/helixis-domain`, `helixis-application`,
   `helixis-persistence`, and `helixis-protocol`
4. keep scheduling inside control-plane initially
5. extract autoscaler and runtime-specific crates only when needed

This avoids a big-bang refactor while still steering the codebase toward a
clean architecture.

## Brainstorming Notes

Ideas worth considering as the codebase evolves:

- a unified internal event enum for task lifecycle changes, even if events are
  persisted only as audit records at first
- pluggable scaling adapters behind a `ScalingAdapter` trait
- optional gRPC between internal services later, while keeping public API HTTP
- per-runtime executor binaries only if dependency footprint becomes painful
- a shared `clock` abstraction to make lease and retry tests deterministic
- a future workflow engine as a separate crate built on top of the same task and
  schedule primitives
- a dedicated sandbox crate once process isolation gives way to microVM or
  hardened container isolation

## Recommendation Summary

The strongest starting point is:

- a Cargo workspace
- thin binaries
- domain and application logic in libraries
- Postgres-backed persistence adapters
- executor runtime abstractions isolated from control-plane code

That gives Helixis room to grow without turning the Rust codebase into a single
hard-to-change service binary.
