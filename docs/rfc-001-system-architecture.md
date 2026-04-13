# RFC-001: Helixis System Architecture

## Status

Draft v1

## Purpose

Helixis is a distributed execution platform that accepts language-specific
artifacts, schedules work via a pull-based protocol, and executes workloads in
isolated runtime environments without requiring users to manage worker fleets.

The system combines three ideas:

- queue semantics for asynchronous task dispatch
- artifact-based execution similar to serverless runtimes
- orchestration and scheduling primitives for retries, timeouts, and future
  workflow support once the core async engine is stable

## Goals

- Run arbitrary functions in multiple languages using a unified execution model.
- Separate control-plane decision making from execution-plane work.
- Support pull-based executors with no inbound network requirement.
- Preserve runtime determinism using immutable artifact versions and runtime
  packs.
- Run on local Docker, EC2, ECS/Fargate, or Kubernetes with minimal behavioral
  drift.
- Keep the v1 control plane operationally simple enough for a small team to
  ship and run.
- Treat all uploaded workloads as untrusted code and design isolation
  boundaries accordingly.

## Non-Goals For v1

- exactly-once execution guarantees
- full DAG/workflow engine parity with Temporal
- workflow orchestration as a first-release feature
- hard microVM isolation in the initial release
- cross-region active/active scheduling
- user-supplied custom OCI images as the primary execution mechanism

## Design Principles

### 1. Pull-Only Executors

Executors always initiate contact using `poll`, `heartbeat`, and completion
calls. This removes inbound connectivity requirements, simplifies NAT/VPC
deployments, and allows environments like Fargate or private clusters to work
without extra coordination channels.

### 2. Deterministic Runtime Binding

Every execution is bound to:

```text
artifact_version + runtime_pack + execution_policy
```

This must fully determine the runtime environment, sandbox parameters, and
dependency resolution behavior.

### 3. Lease-Based Scheduling

Tasks are never permanently assigned on dispatch. They are leased to executors
and can be re-queued if the lease expires or the executor becomes unhealthy.

### 4. Control Plane Simplicity First

For v1, prefer a single source of truth in Postgres plus object storage for
artifacts and logs. Introduce separate brokers or specialized queue systems only
when measured throughput requires them.

### 5. Untrusted Code By Default

Executors run user-uploaded code that must be treated as hostile by default.
This has direct implications:

- runtime packs must be tightly controlled by the platform
- artifacts must be prebuilt and validated before registration
- sandboxes must deny privilege escalation and tightly limit resources
- secrets exposure and network egress must be explicit policy decisions
- even in single-tenant v1, workloads must be isolated from platform internals

## Top-Level Architecture

```text
                           +----------------------+
                           |     Clients / SDKs   |
                           +----------+-----------+
                                      |
                                      v
                      +---------------+----------------+
                      |            API Plane           |
                      | submit / query / auth / admin |
                      +---------------+----------------+
                                      |
                                      v
                      +---------------+----------------+
                      |         Control Plane          |
                      | scheduler / lease mgr / ASG   |
                      +---+------------------------+---+
                          |                        |
                +---------+--------+      +--------+---------+
                | Metadata + Queue |      | Artifact Storage |
                | Postgres         |      | S3/MinIO         |
                +------------------+      +------------------+
                          ^
                          |
               +----------+-----------+
               | Pull-Based Executors |
               | python / node / etc  |
               +----------------------+
```

## Major Runtime Components

### API Plane

Responsibilities:

- authenticate tenants and callers
- validate task submission requests
- resolve artifact metadata and runtime compatibility
- insert tasks into the queueing model
- expose task status and result retrieval APIs
- expose registration and administrative endpoints

Recommended v1 deployment:

- Rust `axum` HTTP service
- stateless replicas behind an ingress or load balancer
- ECS/Fargate and local environments are the first-class deployment targets

### Scheduler

Responsibilities:

- decide which queued task should be leased to which executor capability set
- enforce fairness and quotas
- manage lease expiry and rescheduling
- compute backpressure signals for autoscaling

Important note:

Because executors poll for work, "scheduling" in v1 should happen inside the
poll path or in a closely coupled scheduling service rather than a separate push
dispatcher. This avoids dual ownership of assignment state.

### Autoscaler

Responsibilities:

- consume backlog and runtime-specific throughput metrics
- compute desired executor counts by runtime pack or executor pool
- call environment-specific adapters

The autoscaler should be decoupled from task dispatch. A scaling failure must
not block normal task scheduling.

### Artifact Service

Responsibilities:

- register immutable artifacts and versions
- store package metadata, hashes, size, language, dependency manifest, and
  entrypoint
- provide signed download locations or proxied streams to executors
- support cache validation through content-addressed identifiers

Artifacts should be stored outside Postgres in object storage. Postgres stores
metadata and references only.

### Executors

Responsibilities:

- register supported runtime packs and current capacity
- long-poll for work
- fetch and cache artifacts
- execute within a sandbox
- stream or upload logs
- report completion, failures, and heartbeats

Executors should be dumb about global orchestration and smart about local
runtime management, cache efficiency, and task isolation.

## Data Model

### Core Tables

Recommended control-plane tables for v1:

- `tenants`
- `artifacts`
- `artifact_versions`
- `runtime_packs`
- `tasks`
- `task_attempts`
- `task_leases`
- `executors`
- `executor_heartbeats`
- `task_logs`
- `idempotency_keys`
- `quotas`
- `rate_limits`
- `secrets`
- `log_stream_sessions`

### Task Record

Suggested task fields:

- `task_id`
- `tenant_id`
- `artifact_version_id`
- `artifact_digest`
- `runtime_pack_id`
- `status`
- `priority`
- `rate_limit_key`
- `cancel_requested_at`
- `payload_ref` or inline payload
- `timeout_seconds`
- `max_attempts`
- `current_attempt`
- `idempotency_key`
- `scheduled_at`
- `not_before`
- `created_at`
- `updated_at`
- `result_ref`
- `payload_size_bytes`
- `result_size_bytes`
- `log_size_bytes`
- `logs_truncated`
- `last_error_code`
- `last_error_message`

### Attempt Record

Each retry should produce a `task_attempts` row with:

- `attempt_id`
- `task_id`
- `executor_id`
- `lease_id`
- `started_at`
- `finished_at`
- `exit_reason`
- `resource_usage`
- `logs_ref`
- `result_ref`

This provides a durable audit trail without mutating one task record beyond
recognition.

## Queueing Model

### Recommendation: Postgres-First Queue

Use Postgres as the authoritative queue in v1. Tasks remain in the `tasks`
table and are selected using indexed filters plus `FOR UPDATE SKIP LOCKED`.

Why this is a good v1 tradeoff:

- fewer moving parts
- easier transactional consistency for status and leasing
- simpler debugging and support tooling
- strong fit for moderate throughput systems

Known limits:

- extremely high queue volume may require partitioning or a dedicated broker
- long-running heavy scans must be avoided with careful indexing

### Suggested Queue Access Pattern

The `poll` flow should:

1. validate executor registration and health
2. find candidate tasks matching capabilities and tenant constraints
3. lock one or more rows using `SKIP LOCKED`
4. create or renew a lease record
5. transition task state to `SCHEDULED`
6. return the assignment payload

Indexes matter more than clever code here. The queueing path should be designed
around:

- `status`
- `runtime_pack_id`
- `not_before`
- `priority`
- `tenant_id`

## Executor Protocol

### Registration

Executors register a stable logical identity and an ephemeral process session.

Recommended split:

- `executor_id`: durable machine or deployment identity
- `session_id`: current process instance identity

This makes crash recovery and stale lease detection easier.

### Poll

Polling should be long-poll with a bounded wait, for example 15 to 30 seconds.
The response should be one of:

- assigned task
- no work available
- backoff directive
- re-register required

### Heartbeats

Heartbeats serve two purposes:

- executor liveness
- attempt liveness for running tasks

Heartbeat payloads should include:

- current load
- active attempts
- available worker slots
- cache stats
- runtime pack availability

### Completion

Completion should be idempotent. A duplicate completion message must not corrupt
state if the network retries after a successful commit.

### Cancellation

Cancellation is cooperative in v1:

- control plane marks `cancel_requested_at`
- executor receives cancellation state via poll or heartbeat responses
- runtime supervisor sends termination to the subprocess
- task transitions to `CANCELLED` if termination succeeds before completion

Hard preemption beyond local process termination is deferred to stronger
sandboxing phases.

## Task Lifecycle

Recommended states:

```text
QUEUED
SCHEDULED
RUNNING
SUCCEEDED
FAILED
CANCELLED
TIMED_OUT
DEAD_LETTER
```

Recommended ownership:

- `QUEUED`: API or schedule trigger
- `SCHEDULED`: scheduler/poll assignment transaction
- `RUNNING`: executor when local execution begins
- terminal states: executor plus control-plane validation

### Lease Rules

- default lease duration: 30 seconds for dispatch lease
- running tasks must extend a separate attempt heartbeat or execution lease
- if a lease expires, the attempt is marked lost and the task becomes eligible
  for retry
- only one active lease may exist per attempt

The platform is at-least-once by default. Users must supply idempotency keys or
task-level idempotent logic for externally visible side effects.

## Scheduling Strategy

### Matching

A task can be assigned only if:

- runtime pack matches executor capability
- executor has an available slot
- tenant quota allows execution
- rate-limit policy allows execution
- task is eligible by time and retry policy

### Scoring

Suggested v1 score inputs:

- executor free capacity
- artifact cache locality
- tenant fairness debt
- runtime warmness
- optional region/zone affinity

Use a simple weighted score initially. Resist building a complex market-based
scheduler before there is production evidence it is needed.

### Fairness

Recommended fairness model for v1:

- weighted fair selection across tenants
- per-tenant concurrency caps
- rate-limit aware admission control
- per-runtime global queue limits

This is easier to explain and debug than a highly dynamic priority system.

Recommended execution guarantee model for v1:

- best-effort fair scheduling
- hard configured concurrency caps
- hard configured rate limits
- no reserved capacity guarantees yet

This avoids overpromising deterministic capacity while still giving operators
real control over abuse and saturation.

## Artifact Model

### Artifact Structure

Artifacts should be immutable, content-addressed packages with metadata:

- source language
- runtime pack
- entrypoint
- dependency manifest digest
- package toolchain metadata when provided by external build systems
- package checksum
- creation timestamp

Suggested artifact types:

- zip/tarball bundle for Python and Node
- compiled binary packages for Rust and Go
- optional OCI-like descriptor metadata without requiring full container images

Artifact identity recommendation:

- use a global content-addressed digest as the canonical artifact version ID
- store tenant or project ownership as metadata attached to that digest
- deduplicate identical artifacts across uploads while preserving access control

This is the simplest model with the best cache behavior and integrity story.

### Build Separation

Do not conflate building and running in v1. Helixis should accept already-built
artifacts or externally packaged outputs only. This keeps the runtime path fast,
limits supply-chain complexity inside the platform, and reduces blast radius.

## Runtime Packs

A runtime pack is a versioned execution contract, not just a language string.

Suggested fields:

- `runtime_pack_id`
- language
- language_version
- libc or ABI family
- architecture
- sandbox_kind
- filesystem contract
- network policy
- max supported artifact format version
- deprecation status

Example:

```text
python-3.11-glibc-amd64-proc-v1
node-20-glibc-amd64-proc-v1
rust-1.78-static-amd64-proc-v1
```

## Execution Model

### Executor Internals

Each executor process should contain:

- poller
- capacity manager
- artifact cache manager
- runtime supervisor
- worker pool
- heartbeat loop
- log uploader

### Local Flow

1. poll receives assignment
2. reserve local slot
3. fetch artifact or load from cache
4. materialize execution directory
5. spawn runtime sandbox
6. capture stdout/stderr and structured status
7. enforce timeout and resource limits
8. upload outputs and report terminal status
9. cleanup ephemeral workspace

### Isolation

v1:

- process-level isolation
- cgroups or equivalent limits
- filesystem sandboxing
- execution timeout enforcement
- strict runtime allowlists
- no privilege escalation inside the execution environment
- network egress enabled by default, but still mediated by policy and subject to
  future allowlists

v2:

- Firecracker or microVM-backed runtime packs
- stronger network and kernel isolation

## Storage Decisions

Recommended storage split:

- Postgres for metadata, leases, schedule state, and control-plane state
- S3/MinIO for artifacts, large payloads, logs, and results
- Redis optional for ephemeral rate limiting or hot counters, not as the source
  of truth

Avoid making Redis mandatory in v1 unless there is a concrete performance
reason. It adds operational overhead and dual-write complexity.

## Security Model

Minimum v1 security posture:

- outbound-only executor traffic
- signed executor registration credentials
- tenant-scoped API tokens
- platform-managed secrets injected at execution start, never baked into
  artifacts
- per-task resource limits
- audit trail for artifact publication and task execution
- treat every task as untrusted code with no trust in user-packaged content

Recommended v1 secrets model:

- store encrypted platform-managed secrets in the control plane
- scope secrets at the single tenant's project or environment level
- inject them as environment variables or mounted files at execution start
- redact secret values from logs and result metadata
- design the API so short-lived cloud credentials can be added later without
  changing the task contract

Open security questions for later phases:

- should cloud metadata endpoints always be blocked
- when should outbound egress move from default-allow to policy-driven allowlists
- when should short-lived dynamic credentials be added

## Observability

### Metrics

- queue depth by runtime and tenant
- task latency percentiles
- assignment wait time
- executor slot utilization
- cache hit ratio
- attempt failure reasons
- lease expiry count

### Logs

- structured control-plane logs with request and task correlation IDs
- per-attempt execution logs
- scheduler decision logs for debugging fairness and placement
- live log streaming to clients while attempts are running
- retained post-run logs for historical inspection

Recommended v1 log policy:

- stream logs live over SSE or WebSocket
- persist logs to object storage after attempt completion
- cap retained logs at 50 MiB per attempt, then truncate with an explicit marker

### Tracing

Recommended trace chain:

```text
submit request -> task create -> poll assignment -> attempt start ->
artifact fetch -> runtime spawn -> completion commit
```

## Deployment Modes

### Local

- docker compose or local binaries
- single Postgres
- MinIO for artifact storage
- executor running on the developer machine

### ECS/Fargate

- API, scheduler, autoscaler as services
- executor as per-runtime service or task family
- preferred first cloud deployment target for v1

### Kubernetes

- one deployment per executor pool/runtime pack
- autoscaler maps runtime demand to deployment replicas

### EC2

- systemd or Docker-managed executor agents
- useful for stateful cache-heavy executor pools

### Region Strategy

v1 should be single-region only. Multi-region adds too much coordination
complexity for task routing, artifact locality, and failover relative to the
initial product scope.

## Recommended v1 Scope

To reduce risk, v1 should target:

- Python and Node runtime packs
- async task submission and result retrieval
- retries and timeouts
- tenant quotas and idempotency keys
- priorities, rate limits, and cancellation
- local and ECS/Fargate deployment targets
- externally built artifacts only
- single-tenant operation with untrusted workload isolation
- bounded payload, result, and log sizes
- global content-addressed artifact identities
- live log streaming with retained historical logs

Do not start with:

- workflow DAG authoring
- live interactive shells
- arbitrary custom runtime packs from users
- multi-region failover

## Key Risks

- over-designing the scheduler before baseline throughput measurements exist
- under-specifying idempotency and retry semantics
- mixing build and execution responsibilities too early
- under-investing in isolation even though the platform runs untrusted code

## Recommendation Summary

Build Helixis v1 as:

- a Rust control plane with stateless API services
- Postgres-first transactional scheduling
- S3-compatible artifact and output storage
- pull-based executors grouped by runtime pack
- process-isolated sandboxes with strict leases and heartbeats
- single-region local and ECS/Fargate deployments
- OpenTelemetry plus Prometheus and Grafana observability

Suggested default v1 size limits:

- request payload: 1 MiB max
- execution result: 10 MiB max
- retained logs: 50 MiB per attempt max, then truncate

This provides a production-realistic base without locking the system into an
overly expensive or overcomplicated architecture.
