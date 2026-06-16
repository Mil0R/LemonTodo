#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${LEMONTODO_REGISTER_WASM_DIR:-${ROOT_DIR}/target/register-wasm}"
TARGET_TRIPLE="wasm32-unknown-unknown"
WASM_INPUT="${ROOT_DIR}/target/${TARGET_TRIPLE}/release/lemontodo_register_wasm.wasm"
OUT_JS="register_wasm.js"
OUT_WASM="register_wasm_bg.wasm"

log() {
    printf '[build-register-wasm] %s\n' "$*"
}

fail() {
    printf '%b\n' "$*" >&2
    exit 1
}

require_command() {
    local cmd="$1"
    local install_hint="$2"
    if ! command -v "${cmd}" >/dev/null 2>&1; then
        fail "${cmd} is required.\n${install_hint}"
    fi
}

has_wasm_target() {
    local sysroot
    sysroot="$(rustc --print sysroot)"
    compgen -G "${sysroot}/lib/rustlib/${TARGET_TRIPLE}/lib/libcore-*.rlib" >/dev/null
}

ensure_wasm_target() {
    if has_wasm_target; then
        return 0
    fi

    if command -v rustup >/dev/null 2>&1; then
        log "installing missing ${TARGET_TRIPLE} target via rustup"
        rustup target add "${TARGET_TRIPLE}"
        has_wasm_target && return 0
        fail "failed to install ${TARGET_TRIPLE} with rustup"
    fi

    fail "${TARGET_TRIPLE} target is not installed.
Install it with your Rust toolchain manager before running this script.
If you use rustup: rustup target add ${TARGET_TRIPLE}"
}

create_temp_out_dir() {
    local out_parent
    out_parent="$(dirname "${OUT_DIR}")"
    mkdir -p "${out_parent}"
    mktemp -d "${out_parent}/register-wasm.tmp.XXXXXX"
}

verify_outputs() {
    local dir="$1"
    [[ -f "${dir}/${OUT_JS}" ]] || fail "missing generated asset: ${dir}/${OUT_JS}"
    [[ -f "${dir}/${OUT_WASM}" ]] || fail "missing generated asset: ${dir}/${OUT_WASM}"
}

require_command cargo "Install Rust and Cargo from https://www.rust-lang.org/tools/install"
require_command rustc "Install Rust from https://www.rust-lang.org/tools/install"
require_command wasm-bindgen "wasm-bindgen CLI is required.\nInstall it with: cargo install wasm-bindgen-cli"
ensure_wasm_target

TEMP_OUT_DIR=""
cleanup() {
    if [[ -n "${TEMP_OUT_DIR}" && -d "${TEMP_OUT_DIR}" ]]; then
        rm -rf "${TEMP_OUT_DIR}"
    fi
}
trap cleanup EXIT

cd "${ROOT_DIR}"

log "building lemontodo-register-wasm for ${TARGET_TRIPLE}"
cargo build -p lemontodo-register-wasm --release --target "${TARGET_TRIPLE}"

[[ -f "${WASM_INPUT}" ]] || fail "expected build output not found: ${WASM_INPUT}"

TEMP_OUT_DIR="$(create_temp_out_dir)"
log "generating browser bindings with wasm-bindgen"
wasm-bindgen "${WASM_INPUT}" --target web --out-dir "${TEMP_OUT_DIR}" --out-name register_wasm
verify_outputs "${TEMP_OUT_DIR}"

rm -rf "${OUT_DIR}"
mv "${TEMP_OUT_DIR}" "${OUT_DIR}"
TEMP_OUT_DIR=""

log "WASM assets written to ${OUT_DIR}"
