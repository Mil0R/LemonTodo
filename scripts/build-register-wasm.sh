#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${LEMONTODO_REGISTER_WASM_DIR:-${ROOT_DIR}/target/register-wasm}"
WASM_INPUT="${ROOT_DIR}/target/wasm32-unknown-unknown/release/lemontodo_register_wasm.wasm"

if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -q '^wasm32-unknown-unknown$'; then
    rustup target add wasm32-unknown-unknown
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "wasm-bindgen CLI is required. Install it with:" >&2
    echo "cargo install wasm-bindgen-cli" >&2
    exit 1
fi

cd "${ROOT_DIR}"
cargo build -p lemontodo-register-wasm --release --target wasm32-unknown-unknown
mkdir -p "${OUT_DIR}"
wasm-bindgen "${WASM_INPUT}" --target web --out-dir "${OUT_DIR}" --out-name register_wasm

echo "Register WASM assets written to ${OUT_DIR}"
