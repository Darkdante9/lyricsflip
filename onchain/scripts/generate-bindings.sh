#!/usr/bin/env bash
# generate-bindings.sh — generate TypeScript bindings from the lyricsflip
# contract WASM and write them to frontend/src/lib/stellar/generated/.
#
# Usage:
#   ./scripts/generate-bindings.sh
#
# The script must be run from the repo root or from onchain/.
# It relies on:
#   • Rust + wasm32v1-none target  (for building the WASM)
#   • Stellar CLI                  (for `stellar contract bindings typescript`)
#
# The generated bindings directory is committed so that the frontend can be
# type-checked without a full contract rebuild. CI runs
# scripts/check-bindings-fresh.sh to detect drift between the Rust source and
# the committed bindings.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ONCHAIN_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${ONCHAIN_DIR}/.." && pwd)"
OUT_DIR="${REPO_ROOT}/frontend/src/lib/stellar/generated"

log() { echo "[generate-bindings.sh] $*"; }

# ---------------------------------------------------------------------------
# 1. Build WASM
# ---------------------------------------------------------------------------
log "Building lyricsflip WASM…"
(
  cd "${ONCHAIN_DIR}"
  cargo build --target wasm32v1-none --release 2>&1
)

WASM="${ONCHAIN_DIR}/target/wasm32v1-none/release/lyricsflip.wasm"
if [[ ! -f "${WASM}" ]]; then
  echo "ERROR: ${WASM} not found."; exit 1
fi

# ---------------------------------------------------------------------------
# 2. Generate TypeScript bindings
# ---------------------------------------------------------------------------
log "Generating TypeScript bindings into ${OUT_DIR}…"
mkdir -p "${OUT_DIR}"

stellar contract bindings typescript \
  --wasm "${WASM}" \
  --output-dir "${OUT_DIR}" \
  --overwrite

log "Bindings written to ${OUT_DIR}"
log "Commit the generated files so the frontend can type-check without re-building."
