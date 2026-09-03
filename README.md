# PromptForge

**Local-first Prompt Compiler.** Aus einer kurzen natürlichen Beschreibung
erzeugt PromptForge eine vollständige Prompt-Spezifikation (Prompt IR), einen
bewusst ausführlichen Long Prompt, optimiert ihn token- und semantikorientiert,
verifiziert die Optimierung und liefert einen für ein Ziel-LLM geeigneten
Prompt — ohne dass du Prompt Engineering beherrschen musst.

```text
User Intent → Intent Analysis → Prompt IR → Expansion → Long Prompt
            → Optimization → Verification → Optimized Prompt → Target LLM
```

PromptForge ist **kein Prompt-Rewriter**: Es ist eine Compiler-Pipeline mit
expliziter IR, nachvollziehbaren Optimizer-Passes, strukturierter Verifikation
und Token-Accounting.

## Architektur in einem Satz

Monorepo: **Rust-Core** (IR, Orchestrierung, Optimizer-Passes, Verifikation,
Token-Accounting, Config, Persistenz, CLI/TUI/Service, Logging) + **Python-AI-
Schicht** (Provider-Abstraktion mit `any-llm`, Mock-LLM, LLM-gestützte
Architect-/Optimize-/Verify-Passes), verbunden über **PyO3** mit einem
transaktionalen JSON-Vertrag (ein Aufruf pro LLM-Operation).

Siehe `docs/architecture.md` (vollständige Entscheidungen) und
`docs/api.md` (HTTP-API).

## Repository-Layout

```text
crates/
  pf-core/      IR, Config, Fehler, Token-Accounting, Persistenz, Redaction, Clipboard
  pf-engine/    Pipeline-Orchestrierung, Expansion, Optimizer-Passes, Verifikation, MockBridge
  pf-bridge/    PyO3-Boundary: eingebettetes Python (any-llm/Mock)
  pf-cli/       Binary `prompt-forge`
  pf-tui/       TUI (ratatui)
  pf-service/   HTTP-Service (axum)
python/
  src/promptforge/  Python-AI-Schicht (bridge.py = Einstiegspunkt Rust→Python)
  tests/            pytest/unittest
docs/             architecture.md, api.md, ADRs, Phasenberichte
```

## Installation & Entwicklung

Voraussetzungen: Rust ≥ 1.85 (empfohlen: stable), `uv`, Python ≥ 3.11.

```bash
# 1) Python-Venv (uv) + Python-Paket + any-llm
cd python
uv venv .venv --python /opt/homebrew/bin/python3.14   # oder: uv venv
uv sync                                              # installiert promptforge + any-llm-sdk
cd ..

# 2) Rust bauen (PyO3 baut gegen das Venv-Python)
export PYO3_PYTHON="$(pwd)/python/.venv/bin/python"
cargo build --release

# 3) Tests
cargo test --workspace
cd python && PYTHONPATH=src uv run python -m unittest discover -s tests -p 'test_*.py' && cd ..
```

Hinweis: Ohne Venv fällt die Bridge auf `PF_PYTHON_PATH` oder das Repo-Layout
(`python/src`) zurück; dann funktioniert der deterministische Modus und der
Python-Mock, aber kein echter LLM-Call (any-llm fehlt).

## LLM-Konfiguration

PromptForge unterstützt beliebige OpenAI-kompatible Endpunkte
(lokal: Ollama, LM Studio, llama.cpp; Cloud: beliebige Gateways) und —
ohne Endpunkt — any-llm-Provider über `provider:model`-Syntax.

```bash
# Lokal (z. B. Ollama)
export LLM_ENDPOINT=http://localhost:11434/v1
export LLM_KEY=
export LLM_MODEL=qwen2.5-coder:7b

# Cloud (OpenAI-kompatibel oder any-llm-Provider)
export LLM_ENDPOINT=https://api.openai.com/v1
export LLM_KEY=sk-…
export LLM_MODEL=gpt-4o-mini
# ohne LLM_ENDPOINT:  export LLM_MODEL=anthropic:claude-…  (+ ANTHROPIC_API_KEY)
```

Weitere Variablen: `PF_HOME`, `PF_PROVIDER` (auto|any_llm|mock|none),
`PF_LOG_LEVEL`, `PF_LOG_FORMAT` (text|json), `PF_LOG_RETENTION`,
`PF_PROMPT_LOG` (0|1), `PF_VERIFY_THRESHOLD`, `PF_MAX_ATTEMPTS`,
`PF_SERVICE_HOST/PORT`, `PF_PYTHON_PATH`.

`prompt-forge init` legt `~/.prompt-forge/` (config/prompt/history/cache/
logs/state) samt Default-`config.toml` an.

## CLI Usage

```bash
prompt-forge init
prompt-forge compile "Analysiere diese fünf Papers, vergleiche die Methoden …"
echo "Analysiere diese Papers …" | prompt-forge
prompt-forge compile input.txt
prompt-forge compile input.txt -o result.md        # Prompt in Datei
prompt-forge compile "…" --copy                     # in die Zwischenablage
prompt-forge compile "…" --json                     # komplettes Ergebnis als JSON
prompt-forge compile "…" --no-llm                   # deterministisch (kein LLM)
prompt-forge serve                                  # HTTP-API (127.0.0.1:8770)
prompt-forge tui                                    # interaktive TUI
```

Scriptbar: ohne TTY (Pipe/Umleitung) schreibt `compile` den optimierten
Prompt auf stdout, Statistiken auf stderr. Am TTY erscheint nach dem Lauf ein
Menü: `[1] Copy  [2] Save  [3] Show  [4] Recompile  [q] Quit`.

Exit-Codes: 0 ok · 2 Usage · 3 Config · 4 LLM/Provider · 5 Pipeline/
Verifikation · 6 Persistenz · 7 Infra/Bridge.

## TUI Usage

`prompt-forge tui` — Intent eingeben, Enter kompiliert. Anzeige: Pipeline-
Status, Token-Zahlen, Verifikation, Prompt-Vorschau. `p` schaltet Long/
Optimiert um, `y` kopiert in die Zwischenablage, `s` speichert Artefakte,
`q`/Ctrl-C beendet.

## Service Usage

```bash
prompt-forge serve
curl -s http://127.0.0.1:8770/v1/health
curl -s -X POST http://127.0.0.1:8770/v1/compile \
  -H 'content-type: application/json' \
  -d '{"intent": "Analysiere diese fünf Papers …"}'
```

Endpunkte: `POST /v1/compile`, `POST /v1/optimize`, `POST /v1/verify`,
`POST /v1/execute`, `GET /v1/health` — Details in `docs/api.md`.

## Logging

Rust `tracing` + Python `loguru` (optional) schreiben Rolling-Logs nach
`~/.prompt-forge/logs/` (`prompt-forge.log`, `.1.log`, …; Retention
konfigurierbar). Default menschenlesbar; `PF_LOG_FORMAT=json` für
Service/Machine-Mode.

**Sicherheit:** API-Keys, Authorization-Werte und Tokens werden niemals
geloggt (Redaction auf beiden Seiten). Prompts erscheinen nur mit
`PF_PROMPT_LOG=1` (Opt-in) — standardmäßig loggt PromptForge Metadaten
(request_id, stage, model, duration, token counts, status).

## Tests

- `cargo test --workspace`: IR-Roundtrip, Config-Präzedenz/Redaction,
  Token-Accounting, Optimizer-Passes, Verifikation, Pipeline (MockBridge),
  Persistenz/History, Service-Endpunkte (Mock).
- Python: `cd python && uv run python -m unittest discover -s tests`
  (oder `pytest`): Bridge-JSON-Vertrag, Mock-LLM, Provider-Dispatch,
  any-llm-Fehler-Mapping (Skip ohne Installation).
- E2E Rust→PyO3→Python→MockLLM→Python→Rust: `tests/e2e_python_bridge.rs`
  (überspringt sich mit Hinweis, wenn kein Python/`promptforge` verfügbar).
- Keine echten Cloud-Keys in Tests — überall Mock/Fake-LLM.

## Bekannte Grenzen (v0.1)

- Token-Zählung ist eine Heuristik (Schätzung, transparent gekennzeichnet);
  modellspezifische Tokenizer sind über das `Tokenizer`-Trait erweiterbar.
- Der HTTP-Service nutzt eine prozess-globale Engine-Konfiguration
  (Provider/Modell aus Umgebung), kein Per-Request-Override.
- OS-Keychain-Integration und GUI sind bewusst später (ADR, Architektur
  vorbereitet).
