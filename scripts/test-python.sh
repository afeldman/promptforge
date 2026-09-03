#!/usr/bin/env bash
# Python-Tests (pytest oder unittest-Fallback), lauffähig ohne venv/any-llm.
set -euo pipefail
cd "$(dirname "$0")/../python"
export PYTHONPATH=src
if command -v uv >/dev/null 2>&1 && [ -x .venv/bin/python ]; then
    if .venv/bin/python -c "import pytest" 2>/dev/null; then
        exec .venv/bin/python -m pytest -q
    fi
    exec .venv/bin/python -m unittest discover -s tests -p 'test_*.py'
fi
python3 -m unittest discover -s tests -p 'test_*.py'
