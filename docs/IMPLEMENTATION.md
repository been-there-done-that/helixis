# Helixis Architecture & Implementation Documentation

This document outlines the architecture, end-to-end task lifecycle, implemented phases, and explicitly tracks the "shortcuts" and technical debt accumulated during the MVP generation.

## 1. System Overview

Helixis is a distributed, multi-language task execution orchestrator. It allows a centralized API to act as a task broker, pushing generalized code executions securely to external listening worker agents (executors) without forcing a strict network mesh.

### Core Infrastructure Components
* **Control Plane (`cplane`)**: An Axum-based HTTP server managing the global task queue securely backed by PostgreSQL. Exposes REST endpoints to submit generic execution jobs and fetches status updates.
* **Executor Agents (`executor`)**: Standalone polling daemons programmed in Rust. These agents sit silently in the background routing for specific `RUNTIME_PACK_ID` values. Whenever a matching job surfaces, it securely downloads the S3 execution payload natively, untars the cache locally, uses Tokio processes to isolate the natively configured target (e.g., Python, V8 Engine, Bash scripts), and finally signals job resolution back into the queue!
* **MinIO Object Store**: A local S3 bucket proxy utilized for securely pushing packaged execution caches (typically zipped code bundles natively representing ML models, node packages, or generic workloads) decoupled from API sizes.
* **PostgreSQL Queue**: Stores `tasks`, `tenants`, `artifacts`, and implements secure transactional locks avoiding racing conditions between concurrent polling executor nodes.

---

## 2. End-to-End Orchestrated Task Lifecycle

Whenever an operator/user executes `./scripts/fire_all.sh` (or pushes `curl` requests organically), Helixis transitions logically across the below path:
1. **Queueing**: A `POST /v1/tasks` fires assigning an `artifact_id` pointing to an existing payload natively in Minio. `cplane` sets the task as `Queued`.
2. **Polling**: Executors ping `POST /v1/executors/poll`.
3. **Leasing Locks**: Transactionally, `cplane` runs a deterministic `UPDATE ... FOR UPDATE SKIP LOCKED` query over rows. It transitions the task to `Scheduled`, generates a `lease_id`, and returns it to a single chosen Executor definitively. 
4. **Local Artifact Caching**: The executor connects directly to MinIO, queries missing artifacts bridging locally to `/tmp/helixis/cache/artifacts/`, and decompresses the `tar.gz` bundle.
5. **Dynamic Process Sandboxing**: The `ProcessSandbox` locally resolves environments defined by `EXECUTOR_COMMAND` and triggers `main.py`/`index.js` while pushing stdout back.
6. **Resolution Loop**: The Executor HTTP client pings `POST /v1/tasks/:id/status` transitioning the task row seamlessly to `Succeeded` / `Failed`.

---

## 3. Deliberate Technical Shortcuts & Limitations

While designing the architecture to rapidly produce a fully functional multi-language MVP, we explicitly leveraged structural "under cuts" optimizing for local development loop speed instead of generic scalable Enterprise infrastructure.

If Helixis transitions to a production-ready application cluster, the following components must be refactored:

### Under Cut 1: Hardcoded IP Routing & Local Networking
* **Description:** S3 targets, `cplane` sockets, and HTTP clients bind natively to `http://localhost:9000` / `http://localhost:3000`.
* **Fix needed:** Abstract environments fully requiring `.env` overrides injecting `CPLANE_URL` / `S3_ENDPOINT` natively.
* **Mac Networking:** macOS Docker `host` loops fail silently. We hardcoded `scripts/seed_examples.sh` to use local MinIo exec arrays natively (`docker exec minio mc cp...`) instead of proper `host.docker.internal` DNS bridges generic to enterprise clusters.

### Under Cut 2: Generic Sandbox Processes (`std::process::Command`)
* **Description:** True remote job executors build stringent constraints decoupling file boundaries, local resources, and RAM quotas. The MVP just natively executes Tokio sub-processes directly sharing user directories!
* **Fix needed:** To protect executors from malicious logic payloads pulling sensitive data inside the `/usr/`, `ProcessSandbox` must natively be swapped into securely containerized Cgroups isolating environments (or spawning detached microVMs dynamically via `Firecracker` / `Wasmtime`).

### Under Cut 3: Database Foreign-Key Skeleton Stubbing (Lack of Heartbeat)
* **Description:** We deliberately skipped building the `POST /v1/executors/register` and `POST /executors/heartbeats` API specifications. Because PostgreSQL inherently demands `task_leases` foreign-keys match an active agent directly, we dynamically bypass this by writing raw `INSERT INTO executors ... ON CONFLICT DO NOTHING` inherently whenever an agent leases a task! 
* **Fix needed:** Implement deterministic registration loops routing heartbeats every N seconds! Any task assigned to an unresponsive agent cleanly transitions its `task` back into the `Queued` pool securely. Currently, tasks are permanently held inside `Scheduled` if the executor node crashes independently.

### Under Cut 4: Native S3 Client Virtual Hosting Configs (`object_store`)
* **Description:** By default, standard Amazon S3 dependencies force "Virtual Hosted" configurations attempting to navigate domains (i.e. `bucket.localhost:9000`). Minio running on local topologies fundamentally refuses this topology. 
* **Fix needed:** We natively disabled `.with_virtual_hosted_style_request(false)`. Whenever migrating to real AWS setups organically, this needs to be extracted seamlessly natively or reverted into explicit env-based structures!

### Under Cut 5: Lack of Multi-Tenancy Data Checks 
* **Description:** `cplane` accepts arbitrary UUID references pointing straight at payloads inside MinIO seamlessly mapping tenants. However, `cplane` skips directly validating if the tenant formally owns that `.tar.gz` bucket object! 
* **Fix needed:** Fully flesh out Row-Level Security explicitly filtering artifacts based entirely on `tenant_id` claims pushed inside payloads natively!

---

## 4. Multi-Language Extensibility

In Phase 7, we ensured standard hardcodes weren't restricting tests natively! Executors natively bridge against internal targets via environments overriding logic without recompiling the Rust nodes:
```bash
# Deploys Python
RUNTIME_PACK_ID=python-ml EXECUTOR_COMMAND=python3 EXECUTOR_ENTRYPOINT=main.py cargo run --bin executor 

# Deploys V8 Javascript
RUNTIME_PACK_ID=node-v8 EXECUTOR_COMMAND=node EXECUTOR_ENTRYPOINT=index.js cargo run --bin executor 
```
Instead of `task` tracking metadata organically passing command payloads on demand via JSON fields over HTTP, local Executors act purely as strongly typed instances configured exactly for given worker classes natively! This mirrors advanced generic worker architectures (like RabbitMQ consumers natively mapped against precise deployment arrays).
