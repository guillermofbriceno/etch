#!/usr/bin/env bash
# Integration test orchestrator.
#
# Starts Docker containers (Matrix + Mumble), provisions the Matrix
# server, runs integration tests, and tears everything down.
#
# Usage:
#   ./tests/integration/run.sh          # from repo root or script dir
#   nix develop --command bash -c './tests/integration/run.sh'  # with nix toolchain

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/compose.yml"
PROJECT_NAME="etch-integ"

cleanup() {
    echo "[*] Stopping test containers..."
    docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" down -v --remove-orphans 2>/dev/null || true
}
trap cleanup EXIT

# Start from a clean state
cleanup

echo "[*] Starting test containers..."
docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" up -d

echo "[*] Provisioning Matrix server..."
python3 "$SCRIPT_DIR/provision.py" "$PROJECT_NAME"

echo "[*] Running integration tests..."
cd "$REPO_ROOT"
cargo test -p etch-core --features integration-tests -- --test-threads=1

echo "[*] All integration tests passed."
