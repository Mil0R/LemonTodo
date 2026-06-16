#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="${TMPDIR:-/tmp}/lemontodo-e2e-local"
SERVER_DB="${TMP_DIR}/server.db"
CLIENT_A_DB="${TMP_DIR}/client-a.db"
CLIENT_B_DB="${TMP_DIR}/client-b.db"
SERVER_LOG="${TMP_DIR}/server.log"
SERVER_HOST="${LEMONTODO_SERVER_HOST:-127.0.0.1}"
SERVER_PORT="${LEMONTODO_SERVER_PORT:-8787}"
SERVER_URL="http://${SERVER_HOST}:${SERVER_PORT}"
ACCOUNT_EMAIL="${LEMONTODO_E2E_EMAIL:-you@example.com}"
MASTER_PASSWORD="${LEMONTODO_E2E_MASTER_PASSWORD:-dev-master-password}"
PROJECT_NAME="${LEMONTODO_E2E_PROJECT:-LemonTodo}"
TASK_A_TITLE="${LEMONTODO_E2E_TASK_A:-Task from device A}"
TASK_B_TITLE="${LEMONTODO_E2E_TASK_B:-Task from device B}"
TASK_CONFLICT_A="${LEMONTODO_E2E_CONFLICT_A:-Conflict title from A}"
TASK_CONFLICT_B="${LEMONTODO_E2E_CONFLICT_B:-Conflict title from B}"

if command -v cargo >/dev/null 2>&1; then
    LTD_CMD=(cargo run -q -p lemontodo-tui --)
    SERVER_CMD=(cargo run -q -p lemontodo-server)
else
    echo "cargo is required" >&2
    exit 1
fi

SERVER_PID=""

cleanup() {
    if [[ -n "${SERVER_PID}" ]] && kill -0 "${SERVER_PID}" >/dev/null 2>&1; then
        kill "${SERVER_PID}" >/dev/null 2>&1 || true
        wait "${SERVER_PID}" 2>/dev/null || true
    fi
}

trap cleanup EXIT

log() {
    printf '\n[%s] %s\n' "$(date '+%H:%M:%S')" "$*"
}

run_ltd() {
    "${LTD_CMD[@]}" "$@"
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local message="$3"
    if [[ "${haystack}" != *"${needle}"* ]]; then
        echo "assertion failed: ${message}" >&2
        echo "expected to find: ${needle}" >&2
        exit 1
    fi
}

wait_for_server() {
    local health=""
    for _ in $(seq 1 40); do
        if health="$(curl -fsS "${SERVER_URL}/healthz" 2>/dev/null)"; then
            assert_contains "${health}" "\"status\":\"ok\"" "healthz must report ok"
            return 0
        fi
        sleep 0.25
    done
    echo "server did not become healthy, see ${SERVER_LOG}" >&2
    exit 1
}

start_server() {
    rm -rf "${TMP_DIR}"
    mkdir -p "${TMP_DIR}"

    log "starting local server on ${SERVER_URL}"
    (
        cd "${ROOT_DIR}"
        env \
            LEMONTODO_SERVER_HOST="${SERVER_HOST}" \
            LEMONTODO_SERVER_PORT="${SERVER_PORT}" \
            LEMONTODO_SERVER_DB="${SERVER_DB}" \
            LEMONTODO_ALLOW_REGISTRATION=true \
            "${SERVER_CMD[@]}"
    ) >"${SERVER_LOG}" 2>&1 &
    SERVER_PID=$!

    wait_for_server
}

init_client_a() {
    log "initializing client A"
    run_ltd --db "${CLIENT_A_DB}" init
    run_ltd --db "${CLIENT_A_DB}" sync configure --server-url "${SERVER_URL}" --email "${ACCOUNT_EMAIL}"
    run_ltd --db "${CLIENT_A_DB}" sync register --email "${ACCOUNT_EMAIL}" --master-password "${MASTER_PASSWORD}"
    run_ltd --db "${CLIENT_A_DB}" sync login --email "${ACCOUNT_EMAIL}" --master-password "${MASTER_PASSWORD}"
}

seed_client_a() {
    log "creating initial task on client A"
    run_ltd --db "${CLIENT_A_DB}" project add "${PROJECT_NAME}"
    run_ltd --db "${CLIENT_A_DB}" add "${TASK_A_TITLE}" --project "${PROJECT_NAME}"
    run_ltd --db "${CLIENT_A_DB}" sync now --master-password "${MASTER_PASSWORD}" --apply-safe
}

connect_client_b() {
    log "connecting client B as a second device"
    run_ltd --db "${CLIENT_B_DB}" init
    run_ltd --db "${CLIENT_B_DB}" sync configure --server-url "${SERVER_URL}" --email "${ACCOUNT_EMAIL}"
    run_ltd --db "${CLIENT_B_DB}" sync connect \
        --email "${ACCOUNT_EMAIL}" \
        --master-password "${MASTER_PASSWORD}" \
        --pull \
        --apply-safe

    local listed
    listed="$(run_ltd --db "${CLIENT_B_DB}" list --all)"
    assert_contains "${listed}" "${TASK_A_TITLE}" "client B must receive the first task from A"
}

verify_bidirectional_sync() {
    log "verifying bidirectional sync"
    run_ltd --db "${CLIENT_B_DB}" add "${TASK_B_TITLE}" --project "${PROJECT_NAME}"
    run_ltd --db "${CLIENT_B_DB}" sync now --master-password "${MASTER_PASSWORD}" --apply-safe
    run_ltd --db "${CLIENT_A_DB}" sync now --master-password "${MASTER_PASSWORD}" --apply-safe

    local listed_a
    listed_a="$(run_ltd --db "${CLIENT_A_DB}" list --all)"
    assert_contains "${listed_a}" "${TASK_B_TITLE}" "client A must receive the task created on B"
}

verify_conflict_flow() {
    log "verifying conflict inbox flow"

    local task_id
    task_id="$(
        run_ltd --db "${CLIENT_A_DB}" list --all \
            | rg -o '[0-9a-f]{8}' -m 1
    )"
    if [[ -z "${task_id}" ]]; then
        echo "failed to resolve a task id for conflict scenario" >&2
        exit 1
    fi

    run_ltd --db "${CLIENT_A_DB}" edit "${task_id}" "${TASK_CONFLICT_A}"
    run_ltd --db "${CLIENT_B_DB}" edit "${task_id}" "${TASK_CONFLICT_B}"
    run_ltd --db "${CLIENT_A_DB}" sync now --master-password "${MASTER_PASSWORD}" --apply-safe
    run_ltd --db "${CLIENT_B_DB}" sync now --master-password "${MASTER_PASSWORD}" --apply-safe

    local conflicts
    conflicts="$(run_ltd --db "${CLIENT_B_DB}" sync conflicts)"
    assert_contains "${conflicts}" "status:conflict" "client B must retain a remote conflict entry after concurrent edits"
}

verify_session_flow() {
    log "verifying session/device visibility"

    local whoami sessions
    whoami="$(run_ltd --db "${CLIENT_A_DB}" sync whoami)"
    sessions="$(run_ltd --db "${CLIENT_A_DB}" sync sessions)"

    assert_contains "${whoami}" "${ACCOUNT_EMAIL}" "whoami must report the authenticated account"
    assert_contains "${sessions}" "current" "sessions must include the current session"
}

main() {
    cd "${ROOT_DIR}"

    start_server
    init_client_a
    seed_client_a
    connect_client_b
    verify_bidirectional_sync
    verify_conflict_flow
    verify_session_flow

    log "local client/server e2e passed"
    echo "artifacts kept under ${TMP_DIR}"
}

main "$@"
