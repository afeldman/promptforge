# PromptForge ↔ apfel — Integrationstest (macOS / Apple Silicon)

Reproduzierbarer Smoke-Test der vollständigen PromptForge-Kette gegen einen
**echten lokalen LLM** (`apfel`, Apple Foundation Model):

```text
PromptForge CLI → Rust Engine → Prompt IR → PyO3 → Python → any-llm
→ OpenAI-kompatible API → apfel → Apple Foundation Model → … → Verifier
→ Final Prompt
```

Kein Mock, kein `--no-llm` — der Test verifiziert, dass PromptForge ohne
jeden Provider-spezifischen Code mit einem OpenAI-kompatiblen lokalen
LLM-Endpoint funktioniert.

## Zweck

1. Belegt, dass `apfel` (OpenAI-kompatibler Endpoint für das Apple
   Foundation Model) als `LLM_ENDPOINT` für PromptForge funktioniert.
2. Führt die komplette Pipeline real aus: Architect (IR), Expansion,
   Optimierung, Verifikation, Token-Accounting.
3. Ist als reproduzierbarer Test nach einem Release-Build lauffähig.

## Voraussetzungen

| Voraussetzung | Prüfung |
|---|---|
| macOS (Tahoe 26+) | `uname -s` → `Darwin` (sonst SKIP) |
| Apple Silicon | `uname -m` → `arm64` (sonst SKIP) |
| `apfel` installiert | `command -v apfel` (sonst SKIP) |
| Release-Binary | `target/release/prompt-forge` (sonst FAIL mit Hinweis) |
| `python3` | macOS Command Line Tools (für HTTP-Checks/JSON) |

Der Test startet **keine** Installationen und **keinen** Cargo-Build — er
testet einen bereits gebauten Release-Build.

## Installation von apfel (externe Voraussetzung)

apfel ist kein Teil von PromptForge und wird nicht automatisch installiert:

```bash
brew install apfel          # macOS 26+, Apple Silicon, Apple Intelligence aktiviert
```

Dokumentation/Quelle: https://github.com/Arthur-Ficial/apfel

## Release-Build

Wichtig: PyO3 baut gegen das Python des uv-Venv — vor `cargo build`
einmalig setzen (siehe auch `scripts/env.sh`):

```bash
export PYO3_PYTHON="$(pwd)/python/.venv/bin/python"
cargo build --release
```

## Smoke-Test

```bash
# Startet einen eigenen apfel-Server (Hintergrund), testet, beendet ihn wieder
./tests/providers/apfel/smoke.sh

# Nutzt einen bereits laufenden apfel-Server und beendet ihn NICHT
./tests/providers/apfel/smoke.sh --existing
```

Exit-Codes:

- `0` — `PASS: PromptForge ↔ apfel integration`
- `0` mit `SKIP: …` — Voraussetzung fehlt (kein Fehler, z. B. falsches OS)
- `1` — FAIL (Fehlerdetails auf stderr, Server wird zuverlässig beendet)

## Konfiguration

Standard-Endpoint: `http://127.0.0.1:11434/v1` (von apfel dokumentiert).
Überschreibbar, wenn der Server woanders läuft:

```bash
APFEL_ENDPOINT=http://127.0.0.1:1234/v1 ./tests/providers/apfel/smoke.sh
```

Die Model-ID wird aus `GET /v1/models` ermittelt (bevorzugt
`apple-foundationmodel`, sonst das erste Modell). Explizite Auswahl:

```bash
APFEL_MODEL=apple-foundationmodel ./tests/providers/apfel/smoke.sh
```

`LLM_KEY` wird vom Test nicht gesetzt (apfel benötigt keinen Key);
geerbte Keys werden für den Testlauf neutralisiert. Es werden keine
Credentials in Dateien geschrieben oder ausgegeben.

## Ablauf des Tests

1. Plattform-Checks (macOS, Apple Silicon) — sonst `SKIP`
2. `apfel`-Erkennung — sonst `SKIP`
3. Release-Binary-Prüfung — sonst `FAIL`
4. Server-Lifecycle: startet `apfel --serve` (oder `--existing`), speichert
   PID, wartet auf Bereitschaft (`/health` bzw. `/v1/models`), beendet den
   eigenen Server im Cleanup-Handler (`trap`) — keine verlassenen Prozesse
5. `GET /v1/models` → Model-Discovery (nicht hartcodiert)
6. Direkter OpenAI-Kompatibilitätstest: `POST /v1/chat/completions`
7. PromptForge mit `LLM_ENDPOINT`/`LLM_MODEL` (nur Testprozess)
8. Auswertung der `--json`-Ausgabe (Tokens, Reduktion, Verifikation)

## Erwartetes Ergebnis

```text
PromptForge / apfel integration
--------------------------------
Model: apple-foundationmodel
Input: 25 estimated tokens
Expanded: 246 estimated tokens
Optimized: 105 estimated tokens
Reduction: 57.3% estimated
Verification: pass (semantic 0.90)
Status: PASS
PASS: PromptForge ↔ apfel integration
```

Die Werte stammen aus dem tatsächlichen Lauf und werden nicht hartcodiert.
Token-Zahlen sind **Schätzungen** (Heuristik-Tokenizer von PromptForge).

## Bekannte Einschränkungen

- Das Apple Foundation Model ist klein und befolgt komplexe Meta-Aufgaben
  (JSON-Extraktion/-Optimierung) nur unzuverlässig. Der Test kann bei
  nondeterministischem Modellverhalten mit Exit 4 (Architect lieferte kein
  JSON) fehlschlagen — das ist ein ehrlicher FAIL, kein simulierter PASS;
  Wiederholen ist möglich, die PromptForge-internen Re-Optimize-/Guard-
  Mechanismen arbeiten weiter.
- Der Test erfordert aktive Apple Intelligence auf dem Mac.
- `apfel` v1.5.5 lokal getestet; Kontextfenster 4096 Tokens (macOS 26).
- Kein `jq`/`curl` erforderlich (HTTP via `python3`).
