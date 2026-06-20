#!/bin/bash
# Binary-size comparison: link a hello-world against the FULL runtime
# (default features) vs a SLIM runtime (--no-default-features, plus any
# features passed as arguments), and report the staticlib + executable sizes.
#
# Usage: benchmarks/size.sh [extra-runtime-features]
#   e.g. benchmarks/size.sh stdlib-json   # slim + json only
#
# The slim runtime builds into its own --target-dir so it never clobbers the
# main target/ (and never races another cargo on the shared lock).

set -eu -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
SLIM_TARGET="${TMPDIR:-/tmp}/pyaot_slim_runtime"
OUT_DIR="${TMPDIR:-/tmp}/pyaot_size"
FEATURES="${1:-}"
HELLO="$SCRIPT_DIR/hello.py"

mkdir -p "$OUT_DIR"

echo "== Building release pyaot + full runtime =="
cargo build --release -p pyaot-cli -p pyaot-runtime --manifest-path "$ROOT/Cargo.toml"

echo "== Building slim runtime (--no-default-features${FEATURES:+ --features $FEATURES}) =="
cargo build --release -p pyaot-runtime --no-default-features \
    ${FEATURES:+--features "$FEATURES"} \
    --manifest-path "$ROOT/Cargo.toml" --target-dir "$SLIM_TARGET"

PYAOT="$ROOT/target/release/pyaot"
FULL_LIB="$ROOT/target/release/libpyaot_runtime.a"
SLIM_LIB="$SLIM_TARGET/release/libpyaot_runtime.a"

# `speed-and-size` enables the extra post-link `strip` pass (the default fast
# path skips it), so this measures the minimal achievable executable size.
"$PYAOT" "$HELLO" -o "$OUT_DIR/hello_full" --opt-level speed-and-size --runtime-lib "$FULL_LIB"
"$PYAOT" "$HELLO" -o "$OUT_DIR/hello_slim" --opt-level speed-and-size --runtime-lib "$SLIM_LIB"

size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }

echo ""
echo "| artifact | full | slim${FEATURES:+ (+$FEATURES)} |"
echo "|---|---|---|"
printf "| libpyaot_runtime.a | %s | %s |\n" "$(size_of "$FULL_LIB")" "$(size_of "$SLIM_LIB")"
printf "| hello executable | %s | %s |\n" \
    "$(size_of "$OUT_DIR/hello_full")" "$(size_of "$OUT_DIR/hello_slim")"
