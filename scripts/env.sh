#!/usr/bin/env bash
# PromptForge Dev-Umgebung: PyO3 gegen das uv-Venv (oder Homebrew-Fallback).
# Usage: source scripts/env.sh
set -a

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ -x "$REPO_ROOT/python/.venv/bin/python" ]; then
    export PYO3_PYTHON="$REPO_ROOT/python/.venv/bin/python"
else
    export PYO3_PYTHON="/opt/homebrew/bin/python3.14"
fi
export PF_PYTHON_PATH="${PF_PYTHON_PATH:-$REPO_ROOT/python/src}"

set +a
echo "PYO3_PYTHON=$PYO3_PYTHON"
echo "PF_PYTHON_PATH=$PF_PYTHON_PATH"
