#!/usr/bin/env bash
# Acceptance script for Strong-Typed MIR Rewrite v2 plan.
# Runs at every stage boundary to ensure no regressions.
#
# Stages of the check:
#   1. cargo build --workspace --release
#   2. cargo test --workspace (full test suite)
#   3. Verifier on all examples — expects 0 violations at final-pre-codegen
#   4. Reports per-example delta if any
#
# Exit code: 0 on success, non-zero on first failure.

set -euo pipefail

cd "$(dirname "$0")/.."

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

EXAMPLES_DIR="examples"
PYAOT_BIN="target/release/pyaot"

log() { echo -e "${GREEN}[acceptance]${NC} $*"; }
warn() { echo -e "${YELLOW}[acceptance]${NC} $*"; }
fail() { echo -e "${RED}[acceptance FAIL]${NC} $*"; exit 1; }

# 1. Build (release)
log "stage 1: cargo build --workspace --release"
if ! cargo build --workspace --release 2>&1 | tail -5; then
    fail "build failed"
fi

# 2. Test suite
log "stage 2: cargo test --workspace"
if ! cargo test --workspace 2>&1 | tail -30; then
    fail "test suite failed"
fi

# 3. Verifier sweep of examples
log "stage 3: verifier sweep on examples (final-pre-codegen, hard-error)"

if [ ! -x "$PYAOT_BIN" ]; then
    fail "pyaot binary not found at $PYAOT_BIN"
fi

FAIL_COUNT=0
TOTAL=0
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

for py in "$EXAMPLES_DIR"/test_*.py; do
    name=$(basename "$py" .py)
    TOTAL=$((TOTAL + 1))
    if ! "$PYAOT_BIN" "$py" -o "$TMPDIR/$name" --verify-mir 2>"$TMPDIR/$name.err" >/dev/null; then
        warn "verifier failed on $name"
        tail -10 "$TMPDIR/$name.err"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
done

if [ "$FAIL_COUNT" -gt 0 ]; then
    fail "$FAIL_COUNT/$TOTAL examples failed verifier sweep"
fi

log "ALL CHECKS PASSED ($TOTAL examples verifier-clean)"
