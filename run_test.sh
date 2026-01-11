#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="$ROOT/out"
OUT_PDF="$OUT_DIR/badapple.pdf"
START_URL="https://zeetee1235.github.io/badapple-pdf/play.html"

mkdir -p "$OUT_DIR"

# Build+run encoder to generate PDF
cargo run --release --manifest-path "$ROOT/encoder/Cargo.toml" -- \
  "$ROOT/badapple.mp4" \
  "$ROOT/badapple.ogg" \
  "$OUT_PDF" \
  80 60 30 128 0 \
  "$START_URL"

# Basic sanity checks
pdfinfo "$OUT_PDF"
qpdf --check "$OUT_PDF"
qpdf --show-npages "$OUT_PDF"

printf '\nGenerated PDF: %s\n' "$OUT_PDF"
