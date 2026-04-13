.PHONY: dev-up dev-down seed fire cplane exec-python exec-node exec-bash e2e-all

# Bring up Docker dependencies (MinIO and Postgres)
dev-up:
	docker-compose up -d
	sleep 3

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
	echo "Waiting for services to boot..."; \
	sleep 7; \
	echo "Firing tasks!"; \
	./scripts/fire_all.sh; \
	echo "=========================================="; \
	echo " Press [Ctrl+C] to gracefully shutdown all"; \
	echo "=========================================="; \
	trap 'kill -9 $$CPLANE_PID $$EXEC_PY_PID $$EXEC_ND_PID $$EXEC_SH_PID' INT TERM; \
	wait
