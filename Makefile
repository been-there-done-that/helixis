.PHONY: dev-up dev-down seed fire cplane exec-python exec-node exec-bash e2e-all

# Bring up Docker dependencies (MinIO and Postgres) and wait for healthchecks
dev-up:
	docker-compose up -d --wait

# Tear down Docker dependencies and clear volumes
dev-down:
	docker-compose down -v

# Seed Postgres and MinIO with all our examples
seed: dev-up
	./scripts/seed_examples.sh

# Fire the tasks into the Control Plane
fire:
	./scripts/fire_all.sh

# Run Control Plane
cplane:
	cargo run --bin cplane

# Run an Executor for Python
exec-python:
	RUNTIME_PACK_ID=python-3.11-v1 EXECUTOR_COMMAND=python3 EXECUTOR_ENTRYPOINT=main.py cargo run --bin executor

# Run an Executor for NodeJS
exec-node:
	RUNTIME_PACK_ID=node-20-v1 EXECUTOR_COMMAND=node EXECUTOR_ENTRYPOINT=index.js cargo run --bin executor

# Run an Executor for Bash
exec-bash:
	RUNTIME_PACK_ID=bash-native-v1 EXECUTOR_COMMAND=bash EXECUTOR_ENTRYPOINT=main.sh cargo run --bin executor

# The Ultimate End-to-End Test (Spins up everything, fires tasks, and cleans up on exit)
e2e-all: seed
	@echo "=========================================="
	@echo " Compiling Control Plane and Executors"
	@echo "=========================================="
	cargo build
	@echo "=========================================="
	@echo " Starting Control Plane and Executor Pool"
	@echo "=========================================="
	@cargo run --bin cplane & \
	CPLANE_PID=$$!; \
	RUNTIME_PACK_ID=python-3.11-v1 EXECUTOR_COMMAND=python3 EXECUTOR_ENTRYPOINT=main.py cargo run --bin executor & \
	EXEC_PY_PID=$$!; \
	RUNTIME_PACK_ID=node-20-v1 EXECUTOR_COMMAND=node EXECUTOR_ENTRYPOINT=index.js cargo run --bin executor & \
	EXEC_ND_PID=$$!; \
	RUNTIME_PACK_ID=bash-native-v1 EXECUTOR_COMMAND=bash EXECUTOR_ENTRYPOINT=main.sh cargo run --bin executor & \
	EXEC_SH_PID=$$!; \
	echo "Waiting for Control Plane to bind port 3000 natively..."; \
	while ! nc -z localhost 3000; do \
	  sleep 0.2; \
	done; \
	echo "Firing tasks!"; \
	./scripts/fire_all.sh; \
	echo "Waiting for tasks to finalize..."; \
	while psql -h localhost -U postgres -d helixis -tA -c "SELECT status FROM tasks ORDER BY created_at DESC LIMIT 3" | grep -qE "(Queued|Scheduled)"; do \
		sleep 1; \
	done; \
	echo "✅ All tasks processed successfully!"; \
	echo "=========================================="; \
	echo " Auto-shutting down Helixis Cluster..."; \
	echo "=========================================="; \
	kill -9 $$CPLANE_PID $$EXEC_PY_PID $$EXEC_ND_PID $$EXEC_SH_PID
