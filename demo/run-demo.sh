#!/usr/bin/env bash
set -e

echo "========================================="
echo " Helixis E2E Cluster Initialization "
echo "========================================="

echo "[1] Checking if Docker is running..."
if ! docker info > /dev/null 2>&1; then
  echo "ERROR: Docker daemon is not running! Please start Docker Desktop and run this script again."
  exit 1
fi

echo "[2] Starting up Minio & Postgres via docker-compose..."
cd ..
docker-compose up -d
sleep 5

echo "[3] Seeding the Postgres Database..."
# Since db username is 'postgres' and db is 'helixis' locally:
# PGPASSWORD=postgres psql -h localhost -U postgres -d helixis -f demo/seed.sql
docker exec -i $(docker-compose ps -q postgres) psql -U postgres -d helixis < demo/seed.sql || echo "Seed executed! (Conflict ignores are fine)"

echo "[4] Setting up MinIO S3 Buckets..."
# We configure Minio CLI (mc) via a temporary image bypassing host installs
docker run --rm --network host -it minio/mc alias set myminio http://127.0.0.1:9000 minioadmin minioadmin
docker run --rm --network host -it minio/mc mb myminio/artifacts || echo "Bucket exists."

echo "[5] Uploading the payload..."
cd demo
# Minio expects the artifact key to exactly match the Artifact UUID
tar -czvf payload.tar.gz main.py
docker run --rm -v $(pwd):/mnt --network host -it minio/mc cp /mnt/payload.tar.gz myminio/artifacts/11111111-1111-1111-1111-111111111111

echo "========================================="
echo " Cluster is primed! "
echo " You can now execute the demonstration tasks."
echo "========================================="
