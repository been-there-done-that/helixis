#!/usr/bin/env bash
set -e

echo "--- [Firing Execution Matrix] ---"

fire() {
    local RUNTIME=$1
    local UUID=$2
    
    echo "Submitting $RUNTIME..."
    curl -X POST http://localhost:3000/v1/tasks \
      -H "Content-Type: application/json" \
      -d "{
        \"tenant_id\": \"00000000-0000-0000-0000-000000000001\",
        \"artifact_id\": \"$UUID\",
        \"runtime_pack_id\": \"$RUNTIME\",
        \"priority\": 100,
        \"timeout_seconds\": 60
      }"
    echo ""
}

fire "python-3.11-v1" "22222222-2222-2222-2222-222222222222"
fire "node-20-v1" "33333333-3333-3333-3333-333333333333"
fire "bash-native-v1" "44444444-4444-4444-4444-444444444444"

echo "---------------------------------"
echo "Check your Executor Terminals! They should immediately process the queue!"
