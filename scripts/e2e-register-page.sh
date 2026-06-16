#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="${TMPDIR:-/tmp}/lemontodo-register-page-e2e"
SERVER_DB="${TMP_DIR}/server.db"
SERVER_LOG="${TMP_DIR}/server.log"
SERVER_HOST="${LEMONTODO_SERVER_HOST:-127.0.0.1}"
SERVER_PORT="${LEMONTODO_SERVER_PORT:-8787}"
SERVER_URL="http://${SERVER_HOST}:${SERVER_PORT}"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "skipping register page e2e: wasm-bindgen CLI is not installed" >&2
    echo "install it with: cargo install wasm-bindgen-cli" >&2
    exit 0
fi

if ! rustc --print target-list | grep -q '^wasm32-unknown-unknown$'; then
    echo "skipping register page e2e: rustc does not support wasm32-unknown-unknown" >&2
    exit 0
fi

if ! cargo build -q -p lemontodo-register-wasm --release --target wasm32-unknown-unknown >/dev/null 2>&1; then
    echo "skipping register page e2e: wasm32-unknown-unknown target is not installed" >&2
    echo "install it with: rustup target add wasm32-unknown-unknown" >&2
    exit 0
fi

SERVER_PID=""

cleanup() {
    if [[ -n "${SERVER_PID}" ]] && kill -0 "${SERVER_PID}" >/dev/null 2>&1; then
        kill "${SERVER_PID}" >/dev/null 2>&1 || true
        wait "${SERVER_PID}" 2>/dev/null || true
    fi
}

trap cleanup EXIT

wait_for_server() {
    for _ in $(seq 1 40); do
        if curl -fsS "${SERVER_URL}/healthz" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.25
    done
    echo "server did not become healthy, see ${SERVER_LOG}" >&2
    exit 1
}

cd "${ROOT_DIR}"
rm -rf "${TMP_DIR}"
mkdir -p "${TMP_DIR}"

./scripts/build-register-wasm.sh

env \
    LEMONTODO_SERVER_HOST="${SERVER_HOST}" \
    LEMONTODO_SERVER_PORT="${SERVER_PORT}" \
    LEMONTODO_SERVER_DB="${SERVER_DB}" \
    LEMONTODO_ALLOW_REGISTRATION=true \
    cargo run -q -p lemontodo-server >"${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

wait_for_server
LEMONTODO_E2E_SERVER_URL="${SERVER_URL}" npm run e2e:register
