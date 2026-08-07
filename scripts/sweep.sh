#!/usr/bin/env bash
# Prunes stale/oversized cargo build artifacts from target/ using cargo-sweep.
# Install once: cargo install cargo-sweep
#
# Usage:
#   scripts/sweep.sh              # cap target/ at 2GB, oldest artifacts first
#   scripts/sweep.sh 500          # cap at 500MB
#   scripts/sweep.sh 2000 --dry-run

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
maxsize_mb="${1:-2000}"
shift || true

cargo sweep --maxsize "$maxsize_mb" "$@" "$repo_root"
