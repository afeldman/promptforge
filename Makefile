# PromptForge — Root-Makefile (POSIX-kompatibel, macOS-first).
#
# Orchestriert ausschließlich vorhandene Werkzeuge: cargo (Workspace),
# uv (python/pyproject.toml), tests/providers/apfel/smoke.sh.
# Keine Secrets, kein .env, keine User-Daten (nie ~/.prompt-forge anfassen).

.DEFAULT_GOAL := help

# --- Umgebung ---
# Python für PyO3: bevorzugt das uv-Venv (python/.venv), sonst Homebrew
# python3.14 (macOS), sonst python3 aus PATH. Nur als Build-Env für cargo.
PY := $(shell { \
    if [ -x "$(CURDIR)/python/.venv/bin/python" ]; then \
        printf '%s' "$(CURDIR)/python/.venv/bin/python"; \
    elif [ -x /opt/homebrew/bin/python3.14 ]; then \
        printf '%s' "/opt/homebrew/bin/python3.14"; \
    else \
        printf '%s' "python3"; \
    fi; })
export PYO3_PYTHON := $(PY)

help: ## Zeigt diese Hilfe
	@printf '\nPromptForge\n\n'
	@printf 'Usage:\n'
	@printf '  make setup       Set up development environment (uv; macOS: brew apfel falls fehlt)\n'
	@printf '  make setup-python  Python environment via uv (alle Plattformen)\n'
	@printf '  make setup-macos   macOS checks: Homebrew, Apple Silicon, apfel\n'
	@printf '  make build       Build release binary (target/release/prompt-forge)\n'
	@printf '  make test        Run all tests (Rust + Python)\n'
	@printf '  make test-rust   Run cargo test --workspace\n'
	@printf '  make test-python Run Python tests via uv (pytest)\n'
	@printf '  make lint        Run clippy (warnings = Fehler)\n'
	@printf '  make fmt         Check formatting (cargo fmt)\n'
	@printf '  make test-apfel  Run macOS/apfel integration test (real LLM)\n'
	@printf '  make verify      fmt + lint + test + build (+ test-apfel auf macOS/arm64)\n'
	@printf '  make clean       Remove build artifacts (cargo clean)\n'

# --- Setup ---

setup: setup-python ## Bereitet die Entwicklungsumgebung vor (uv; macOS zusätzlich apfel)
	@case "$$(uname -s)" in \
	    Darwin) $(MAKE) setup-macos ;; \
	    *) printf '==> Nicht-macOS: brew/apfel-Setup übersprungen\n' ;; \
	esac

setup-python: ## Python-Umgebung via uv (alle Plattformen; idempotent)
	@command -v uv >/dev/null 2>&1 || { printf 'ERROR: uv is required for the Python environment.\nInstall uv first (https://docs.astral.sh/uv/).\n' >&2; exit 1; }
	@printf '==> uv sync (python/)\n'
	@cd python && uv sync
	@printf '==> Checks:\n'
	@cd python && uv run python --version
	@cd python && uv run python -c "import any_llm; print('any-llm', any_llm.__version__)"
	@cd python && uv run python -c "import loguru; print('loguru OK')"

setup-macos: ## macOS: Homebrew/Apple-Silicon prüfen, apfel sicherstellen (startet apfel NICHT)
	@case "$$(uname -s)" in \
	    Darwin) : ;; \
	    *) printf 'SKIP: setup-macos is only for macOS\n'; exit 0 ;; \
	esac
	@command -v brew >/dev/null 2>&1 || { printf 'ERROR: Homebrew is required for macOS setup.\nInstall Homebrew first.\n' >&2; exit 1; }
	@case "$$(uname -m)" in \
	    arm64) : ;; \
	    *) printf 'SKIP/WARNING: apfel requires Apple Silicon (aktuell: %s) — allgemeine Installation bleibt unverändert\n' "$$(uname -m)"; exit 0 ;; \
	esac
	@if command -v apfel >/dev/null 2>&1; then \
	    printf 'apfel already installed: %s\n' "$$(apfel --version 2>/dev/null | head -1)"; \
	else \
	    printf '==> brew install apfel\n'; \
	    brew install apfel; \
	fi
	@command -v apfel >/dev/null 2>&1 && apfel --help >/dev/null 2>&1 && printf '==> apfel verfügbar (wird nicht gestartet; Server startet der Test)\n'

# --- Build ---

build: ## Erzeugt Release-Binary target/release/prompt-forge
	@printf '==> cargo build --release (PYO3_PYTHON=%s)\n' "$(PYO3_PYTHON)"
	@cargo build --release
	@test -x target/release/prompt-forge && printf 'OK: target/release/prompt-forge\n' || { printf 'ERROR: Binary fehlt nach Build\n' >&2; exit 1; }

# --- Tests ---

test: test-rust test-python ## Führt Rust- und Python-Tests aus
	@printf '==> Alle Tests erfolgreich\n'

test-rust: ## cargo test --workspace (hermetisch: LLM_*-Env scrubben + seriell wegen Env-Tests)
	@printf '==> cargo test --workspace\n'
	@env -u LLM_ENDPOINT -u LLM_KEY -u LLM_MODEL -u PF_PROVIDER cargo test --workspace -- --test-threads=1

test-python: ## Python-Tests via uv (pytest)
	@printf '==> uv run pytest (python/)\n'
	@cd python && uv run pytest

# --- Format / Lint ---

fmt: ## Prüft Formatierung (cargo fmt --check)
	@printf '==> cargo fmt --all -- --check\n'
	@cargo fmt --all -- --check

lint: ## Clippy (Warnings = Fehler)
	@printf '==> cargo clippy --workspace --all-targets -- -D warnings\n'
	@cargo clippy --workspace --all-targets -- -D warnings

# --- apfel-Integrationstest (echter LLM-Pfad, macOS/Apple Silicon) ---

test-apfel: build ## Führt tests/providers/apfel/smoke.sh aus (Default-Modus; eigenständiger Server-Lifecycle)
	@printf '==> tests/providers/apfel/smoke.sh\n'
	@./tests/providers/apfel/smoke.sh

# --- Verifikation / Clean ---

verify: ## Kompletter Qualitätscheck: fmt, lint, test, build; auf macOS/arm64 + apfel zusätzlich test-apfel
	@$(MAKE) fmt
	@$(MAKE) lint
	@$(MAKE) test
	@$(MAKE) build
	@case "$$(uname -s)$$(uname -m)" in \
	    Darwinarm64) \
	        if command -v apfel >/dev/null 2>&1; then \
	            $(MAKE) test-apfel; \
	        else \
	            printf 'SKIP: apfel nicht installiert (make setup-macos installiert es) — Integrationstest übersprungen\n'; \
	        fi ;; \
	    *) printf 'SKIP: apfel-Integrationstest erfordert macOS auf Apple Silicon — übersprungen\n' ;; \
	esac

clean: ## Entfernt Build-Artefakte (nur Build-Artefakte; niemals ~/.prompt-forge o. ä.)
	@printf '==> cargo clean\n'
	@cargo clean
	@printf 'Clean: target/ entfernt. Python-Env (.venv) bleibt erhalten.\n'

.PHONY: help setup setup-python setup-macos build test test-rust test-python fmt lint test-apfel verify clean
