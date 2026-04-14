![Helixis Logo](./docs/logo-transparent.png)

# Helixis

**Helixis** is a high-performance, distributed, multi-language execution platform built in Rust. It generalizes task orchestration by leveraging a robust control plane and secure, dynamic sandboxes for diverse runtimes, completely unifying the observability of disparate workloads.

## 🚨 The Problem

Modern architectures frequently rely on polyglot workloads. You might have Python scripts for data processing, Node.js scripts for API invocations, and Bash scripts for infrastructure automation. 

Orchestrating these isolated tasks usually leads to a scattered mess:
1. **Siloed Execution:** Running disparate jobs via fragmented cron schedules or heavy, complex orchestration platforms like Apache Airflow.
2. **Operational Blindspots:** Tracing a workflow that shifts from a Node service into a Python worker involves reading entirely separate server logs.
3. **Scaling Bottlenecks:** Managing the environment dependencies tightly couples business logic to the underlying machines.

## 💡 The Solution

Helixis completely abstracts runtime execution. You submit a task payload; Helixis guarantees it runs in isolated, language-specific sandboxes and aggregates the resulting state.

- **Unified Control Plane:** A single point of API entry that handles task queueing, state persistence, and dispatch.
- **Dynamic Executors:** Lightweight worker nodes that spin up the right runtime (Python, Node, Bash) on demand.
- **Deep Observability:** Fully instrumented with OpenTelemetry. A Python failure or a Node bottleneck emits unified distributed traces directly directly to your observability backend.

## ✨ Key Features

- **Multi-Language Execution:** Production-ready sandboxes capable of securely running Python 3, Node.js 20, and native Shell scripts.
- **Distributed Architecture:** A scalable Control Plane (`cplane`) orchestrating a horizontally scaling fleet of Executor workers (`executor`).
- **Data Persistence:** Backed by PostgreSQL for immutable task state tracking and MinIO (S3) for blob storage of artifacts and logs.
- **Enterprise-ready Health Checks:** Built-in liveness and readiness endpoints ensuring smooth deployments via Kubernetes or Nomad.

## 🏗️ Architecture Layout

Helixis separates concerns across highly modular, specialized crates:

- **`apps/cplane`**: The Control Plane handling task orchestration, scheduling, and dispatch via API.
- **`apps/executor`**: Scalable worker nodes providing isolated, runtime-specific environments.
- **`crates/telemetry`**: The telemetry bridge capturing spans via OpenTelemetry specifications.
- **`crates/persistence`**: Robust storage abstractions mapping task models to persistent storage using SQLx (PostgreSQL) & Object Stores (MinIO/S3).

## 🚀 Getting Started

Ensure you have [Docker](https://docs.docker.com/engine/install/) and [Rust](https://rustup.rs/) installed.

### 1. Bring up dependencies
Helixis requires PostgreSQL for persistence and MinIO (S3) for storage.
```bash
make dev-up
```

### 2. Seed Example Data
Populate your database and object storage with testing data ranging across Python, Node, and bash workloads.
```bash
make seed
```

### 3. Run the Control Plane
Start the core orchestrator API.
```bash
make cplane
```

### 4. Spawning Executor Nodes
You can launch Executors dedicated to specific languages. Run these in separate terminal windows:
```bash
make exec-python  # Python sandbox
make exec-node    # Node.js sandbox
make exec-bash    # Bash sandbox
```

### 5. Fire Tasks!
Once your control plane and executors are ready, dispatch the seeded jobs.
```bash
make fire
```

---

*For full end-to-end testing which spins up the control plane, all executors, automatically fires tasks, and performs teardown validation, run `make e2e-all`.*
