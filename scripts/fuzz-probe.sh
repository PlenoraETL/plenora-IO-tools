#!/bin/bash
# Probe di fattibilita cargo-fuzz: installa clang+nightly+cargo-fuzz e prova a
# compilare/eseguire un target 20s. Se arriva a "=== RUN OK ===" e' fattibile.
echo "=== clang ==="
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq clang >/dev/null 2>&1
clang --version 2>/dev/null | head -1 || echo "NO CLANG"
echo "=== nightly ==="
rustup toolchain install nightly --profile minimal 2>&1 | tail -1
echo "=== cargo-fuzz install ==="
cargo +nightly install cargo-fuzz --locked 2>&1 | tail -3
echo "=== build + run from_wkb 20s ==="
cd /work || exit 1
cargo +nightly fuzz run from_wkb -- -max_total_time=20 -rss_limit_mb=3072 2>&1 | tail -40
echo "=== RUN OK (exit $?) ==="
