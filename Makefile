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
	@printf '  make test        Run all tests (Rust + Python + deterministischer Compiler-Smoke)\n'
	@printf '  make test-rust   Run cargo test --workspace\n'
	@printf '  make test-python Run Python tests via uv (pytest)\n'
	@printf '  make test-compiler  Deterministic compiler smoke (CompilationResult, --no-llm)\n'
	@printf '  make lint        Run clippy (warnings = Fehler)\n'
	@printf '  make fmt         Check formatting (cargo fmt)\n'
	@printf '  make test-apfel  Run macOS/apfel integration test (real LLM, stochastisch)\n'
	@printf '  make apfel-start  Start local apfel server in background (macOS/arm64; idempotent)\n'
	@printf '  make apfel-stop   Stop the apfel server started by apfel-start\n'
	@printf '  make apfel-status Show apfel install/running/health/model status\n'
	@printf '  make apfel        Alias for apfel-status\n'
	@printf '  make verify      fmt + lint + test + build (deterministisch; apfel NICHT enthalten)\n'
	@printf '  make verify-all  verify + test-apfel (opt-in, echter LLM auf macOS/arm64)\n'
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

test: test-rust test-python test-compiler ## Führt alle deterministischen Tests aus (Rust + Python + Compiler-Smoke)
	@printf '==> Alle Tests erfolgreich\n'

test-rust: ## cargo test --workspace (hermetisch: LLM_*-Env scrubben + seriell wegen Env-Tests)
	@printf '==> cargo test --workspace\n'
	@env -u LLM_ENDPOINT -u LLM_KEY -u LLM_MODEL -u PF_PROVIDER cargo test --workspace -- --test-threads=1

test-python: ## Python-Tests via uv (pytest)
	@printf '==> uv run pytest (python/)\n'
	@cd python && uv run pytest

test-compiler: build ## Deterministischer Compiler-Smoke gegen das Release-Binary (CompilationResult, --no-llm)
	@printf '==> tests/compiler-smoke.sh\n'
	@./tests/compiler-smoke.sh

# --- Format / Lint ---

fmt: ## Prüft Formatierung (cargo fmt --check)
	@printf '==> cargo fmt --all -- --check\n'
	@cargo fmt --all -- --check

lint: ## Clippy (Warnings = Fehler)
	@printf '==> cargo clippy --workspace --all-targets -- -D warnings\n'
	@cargo clippy --workspace --all-targets -- -D warnings

# --- apfel-Integrationstest (echter LLM-Pfad, macOS/Apple Silicon) ---

# Endpoint-Konvention (APFEL_ENDPOINT wie in tests/providers/apfel/smoke.sh).
APFEL_ENDPOINT ?= http://127.0.0.1:11434/v1

apfel: apfel-status ## Alias für apfel-status

apfel-start: ## Startet apfel --serve im Hintergrund (macOS/arm64; idempotent, wartet auf /health)
	@case "$$(uname -s)$$(uname -m)" in \
	    Darwinarm64) \
	        APFEL_ENDPOINT="$(APFEL_ENDPOINT)" scripts/apfel.sh start ;; \
	    *) printf 'SKIP: apfel lifecycle is macOS/Apple Silicon only\n' ;; \
	esac

apfel-stop: ## Beendet den von apfel-start verwalteten apfel-Server (kein Fehler, wenn schon beendet)
	@case "$$(uname -s)$$(uname -m)" in \
	    Darwinarm64) \
	        APFEL_ENDPOINT="$(APFEL_ENDPOINT)" scripts/apfel.sh stop ;; \
	    *) printf 'SKIP: apfel lifecycle is macOS/Apple Silicon only\n' ;; \
	esac

apfel-status: ## Zeigt apfel-Installation, Serverstatus, Health, Endpoint, Modell
	@case "$$(uname -s)$$(uname -m)" in \
	    Darwinarm64) \
	        APFEL_ENDPOINT="$(APFEL_ENDPOINT)" scripts/apfel.sh status ;; \
	    *) printf 'apfel lifecycle is macOS/Apple Silicon only\n' ;; \
	esac

test-apfel: build ## Führt tests/providers/apfel/smoke.sh aus (Default-Modus; eigenständiger Server-Lifecycle)
	@printf '==> tests/providers/apfel/smoke.sh\n'
	@./tests/providers/apfel/smoke.sh

# --- Verifikation / Clean ---
#
# Design-Entscheidung (v0.2 Phase 1): `make verify` ist DETERMINISTISCH und
# hängt bewusst NICHT von der stochastischen Qualität des Apple Foundation
# Model ab (der apfel-E2E kann trotz grüner Gates real fehlschlagen, wenn das
# Modell degeneriert — legitimes Ergebnis laut tests/providers/apfel/README).
# Der echte LLM-Pfad bleibt als opt-in `make verify-all`/`make test-apfel`
# verfügbar. Begründung: reproduzierbares Gate für CI/Entwicklung.

verify: ## Deterministischer Komplettcheck: fmt, lint, test (inkl. Compiler-Smoke), build
	@$(MAKE) fmt
	@$(MAKE) lint
	@$(MAKE) test
	@$(MAKE) build
	@printf '==> verify OK (deterministisch; apfel-Integration separat via make verify-all / make test-apfel)\n'

verify-all: ## verify + echter apfel-Integrationstest (macOS/arm64; stochastisch)
	@$(MAKE) verify
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

.PHONY: help setup setup-python setup-macos build test test-rust test-python test-compiler fmt lint apfel apfel-start apfel-stop apfel-status test-apfel verify verify-all clean
