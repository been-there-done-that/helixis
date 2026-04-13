#!/usr/bin/env bash
set -e

echo "--- [Helixis Multi-Language Seed] ---"
echo "[1] Establishing Buckets..."
export MC_HOST_myminio=http://minioadmin:minioadmin@127.0.0.1:9000
docker run --rm --network host minio/mc mb myminio/artifacts || true

echo "[2] Seeding Base Tenant & Runtime Packs..."
export PGPASSWORD=postgres
psql -h localhost -U postgres -d helixis <<EOF
INSERT INTO tenants (id, name) VALUES ('00000000-0000-0000-0000-000000000001', 'Demo Tenant') ON CONFLICT DO NOTHING;

INSERT INTO runtime_packs (id, language, language_version, sandbox_kind) VALUES 
('python-3.11-v1', 'python', '3.11', 'proc'),
('node-20-v1', 'node', '20.19', 'proc'),
('bash-native-v1', 'bash', '5.2', 'proc')
ON CONFLICT DO NOTHING;
EOF

echo "[3] Building & Uploading Example Artifacts..."

upload_artifact() {
    local PATH_NAME=$1
    local RUNTIME_PACK=$2
    local UUID=$3
    local ENTRY=$4

    echo " -> Compiling $PATH_NAME [$RUNTIME_PACK]..."
    tar -czvf "examples/${PATH_NAME}.tar.gz" -C "examples/${PATH_NAME}" $ENTRY > /dev/null
    
    echo " -> Pushing $UUID to MinIO..."
    docker run --rm -v $(pwd)/examples:/mnt --network host minio/mc cp "/mnt/${PATH_NAME}.tar.gz" myminio/artifacts/$UUID > /dev/null
    
    echo " -> Mapping DB reference..."
    psql -h localhost -U postgres -d helixis -c "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ('$UUID', '00000000-0000-0000-0000-000000000001', 'hash-$UUID', '$RUNTIME_PACK', '$ENTRY', 1024) ON CONFLICT DO NOTHING;" > /dev/null
}

upload_artifact "python-ml" "python-3.11-v1" "22222222-2222-2222-2222-222222222222" "main.py"
upload_artifact "js-v8" "node-20-v1" "33333333-3333-3333-3333-333333333333" "index.js"
upload_artifact "sh-bin" "bash-native-v1" "44444444-4444-4444-4444-444444444444" "main.sh"

echo "------------------------------------"
echo "Seed Complete! Run these in parallel to test:"
echo "Terminal 1: RUNTIME_PACK_ID=python-3.11-v1 EXECUTOR_COMMAND=python3 EXECUTOR_ENTRYPOINT=main.py cargo run --bin executor"
echo "Terminal 2: RUNTIME_PACK_ID=node-20-v1 EXECUTOR_COMMAND=node EXECUTOR_ENTRYPOINT=index.js cargo run --bin executor"
echo "Terminal 3: RUNTIME_PACK_ID=bash-native-v1 EXECUTOR_COMMAND=bash EXECUTOR_ENTRYPOINT=main.sh cargo run --bin executor"
