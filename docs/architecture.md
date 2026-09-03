# PromptForge — Architektur & Entscheidungen (v0.1)

Status: Implementierungsgrundlage v0.1 — 2026-09-03

## 0. Kontext und Ziel

PromptForge ist ein **local-first Prompt Compiler**. Aus einer kurzen
natürlichen Beschreibung („User Intent") wird über eine Compiler-Pipeline eine
vollständige Prompt-Spezifikation (Prompt IR), daraus ein bewusst ausführlicher
Long Prompt, daraus ein optimierter Prompt erzeugt, die Optimierung verifiziert
und schließlich ein für ein Ziel-LLM geeigneter Prompt ausgegeben.

PromptForge ist **kein Prompt-Rewriter**: Die Architektur ist eine
mehrstufige, nachvollziehbare Pipeline mit expliziten Stadien, explizitem IR,
eigenem Optimizer-Pass-System, eigener Verifikation und Token-Accounting.

Leitprinzipien: saubere Architektur, klare Interfaces, Testbarkeit, lokale
Nutzung, reproduzierbares Verhalten, gute CLI/TUI-UX, Erweiterbarkeit
(weitere Provider/Tokenizer/Optimizer/Verification-Engines/GUI), keine
unnötige Komplexität.

## 1. Grundsatzentscheidungen

| ID | Entscheidung | Begründung |
|----|--------------|------------|
| D-01 | **Monorepo**: Rust-Workspace (Cargo) + Python-Paket (uv) in einem Git-Repo | gemeinsame Versionierung, eine Toolchain-Konvention |
| D-02 | **Rust = Core** (IR, Orchestrierung, deterministische Optimizer-Passes, Verifikation, Token-Accounting, Config, Persistenz, CLI/TUI/Service, Logging); **Python = LLM-/AI-Schicht** (Provider, Architect-LLM, Optimize-LLM, Verify-LLM) | deterministische, testbare Kernlogik in Rust; LLM-Ökosystem in Python |
| D-03 | **Sprachgrenze = transaktionale Einbahn**: ein Rust→Python-Aufruf pro LLM-Operation mit JSON-Request/JSON-Response; keine feingranularen Roundtrips | minimale Kopplung, klare Fehlergrenze, testbar mit Mock |
| D-04 | **PyO3 mit eingebettetem CPython** im Rust-Binary (Embed-Mode) + **cdylib-Extension** `pf_bridge` für Python-seitige Tests | Spec verlangt PyO3-Kommunikation; beide Richtungen werden getestet |
| D-05 | **Provider-Abstraktion über Endpunkt**: `LLM_ENDPOINT` + `LLM_KEY` + `LLM_MODEL`; any-llm `create_openai_compatible()` für beliebige OpenAI-kompatible Endpunkte; kein Provider-Wissen im Rust-Core | lokale (Ollama/LM Studio/llama.cpp) wie Cloud-Endpunkte ohne Codeänderung |
| D-06 | **Prompt-IR in Rust** (serde, JSON-Serialisierung), nicht an Provider gebunden | zentrale, versionierte Schnittstelle aller Stadien |
| D-07 | Optimierung als **Pipeline nachvollziehbarer Passes** (deterministische Rust-Passes + optional LLM-Pass über Python), Verifikation strukturiert mit Retry-Loop und harten Limits | keine Black Box; Fehler führen zu definiertem Re-Optimize mit Feedback |
| D-08 | Runtime-User-Home `~/.prompt-forge/` (config, prompt, history, cache, logs, state), über `PF_HOME` übersteuerbar; App legt Verzeichnisse selbst an | Spec §14 |
| D-09 | Projekt-Repo liegt in `~/Projects/priv/forge/PromptForge` (laut Auftragskopf `~/.prompt-forge` — dort existiert kein Projekt; §14 definiert `~/.prompt-forge` als Runtime-Home. Der Auftragskopf basierte auf einer falschen Prämisse; §14 + vorhandenes leeres Projektverzeichnis sind maßgeblich) | Analyse Phase 0 |
| D-10 | Secrets niemals loggen: Redaction-Schicht auf beiden Seiten; Prompt-Inhalte standardmäßig nicht in Logs (opt-in `prompt_log`) | Spec §16/§17 |
| D-11 | Sync-Core-Engine; CLI/TUI/Service wrappen Blocking-Aufrufe in Threads/Tasks | einfache, deterministische Pipeline; parallele LLM-Aufrufe später möglich |
| D-12 | Fehler als typisierte `PfError`-Familie (thiserror) mit stabilem `kind`-Feld für Exit-Codes/API | Spec §19 |
| D-13 | **Formatneutrales `CompilationResult`** (pf-core) als einziges Engine-Ergebnis; **Serializer-Layer** (pf-core, Rust): `OutputFormat` (text/json/yaml/toon) + `PromptSerializer`-Trait; text = ausführbarer Prompt, json/yaml/toon = Envelope desselben Datenmodells; `--format`/`--json`-Alias | v0.2 Design §4/§5; Determinismus, keine zweite Pipeline, Serializer ohne LLM |
| D-14 | **Optimization Engine (v1.0)**: eine Optimize-Stufe wird zur Kandidaten-Engine. Deterministische Strategien `redundancy`/`instruction`/`structural`/`semantic`/`combined` in Rust (`pf-engine::optimization`); LLM-Kompression als zusätzlicher Kandidat `llm`. Jeder Kandidat: Hygiene → Guard (Recovery, gemessen) → strukturelle Verifikation → Scoring; Auswahl des besten gültigen (niemals größer als Input). `OptimizationReport` (additiv in `CompilationResult`): status (`optimized`/`no_improvement`/`degraded`), candidates (tokens/semantic/guard/verification), selected, score. `QualityMetrics` additiv: `constraint_preservation`, `technical_token_preservation`, `redundancy_removed`, `instruction_quality`, `output_contract_quality` | v1.0 Optimization Engine; Caveman-Prinzipien (nur Konzepte: geschützte technische Spans + Prosa-Reduktion, eigene Implementierung); optimale Erhaltung, keine „Zusammenfassung als Optimierung" |
| D-15 | **Guard = Recovery, nicht primäre Optimierung** (v1.0): Guard-Pass bleibt Sicherheitsnetz; Guard-Ereignisse werden pro Kandidat gemessen (`guard_recovered_atoms`, `guard_recovery_ratio`, `pre_guard_tokens`/`output_tokens`) und fließen negativ ins Scoring. Negative Optimierung wird nie als Erfolg gemeldet: kein Kandidat größer als Input; sonst `no_improvement` mit Original | v1.0 §7/§8; ehrliche Metriken |

## 2. Repository-Layout

```
PromptForge/
├── Cargo.toml                  # Cargo-Workspace
├── .cargo/config.toml          # PYO3_PYTHON (venv-gebunden), Umgebungs-Setup
├── rustfmt.toml / .gitignore / .editorconfig
├── README.md
├── docs/
│   ├── architecture.md         # dieses Dokument
│   ├── api.md                  # HTTP-API-Referenz
│   ├── adr/                    # ADRs für Einzelentscheidungen
│   └── phase-reports/          # technische Statusberichte je Phase
├── crates/
│   ├── pf-core/                # IR, Config, Fehler, Token-Accounting, Persistenz, Redaction
│   ├── pf-engine/              # Pipeline-Orchestrierung, Optimizer-Passes, Verifikation
│   ├── pf-bridge/              # PyO3: eingebettetes Python (LLM-Boundary)
│   ├── pf-cli/                 # Binary `prompt-forge` (+ Bibliothek für Tests)
│   ├── pf-tui/                 # ratatui-TUI (Bibliothek; von pf-cli gerufen)
│   └── pf-service/             # axum-HTTP-Service (Bibliothek; von pf-cli gerufen)
├── python/
│   ├── pyproject.toml          # uv-Projekt „promptforge"
│   ├── .venv/                  # (gitignored)
│   ├── src/promptforge/
│   │   ├── bridge.py           # Einstiegspunkt Rust→Python (JSON-in/JSON-out)
│   │   ├── llm/                # Provider-Abstraktion, any-llm, Mock, Fehler-Mapping
│   │   ├── architect.py        # LLM: Intent → Prompt-IR
│   │   ├── optimize.py         # LLM: Long Prompt → optimierter Prompt
│   │   ├── verify.py           # LLM: semantische Verifikation
│   │   ├── config.py / logging.py / errors.py
│   └── tests/                  # pytest
├── scripts/                    # env.sh, test-Runner, E2E-Skripte
└── tests/                      # Rust-Integrationstests (repo-weit, ggf. Shell-E2E)
```

Der Workspace hat **einen binären Einstieg** (`pf-cli` → `prompt-forge`) mit
den Subcommands `init | compile | serve | tui`. `pf-tui` und `pf-service`
sind Bibliotheken, damit CLI, TUI und Service **dieselbe Engine** benutzen
(keine doppelte Business-Logik); eine spätere GUI wird ebenfalls nur Client
der Engine bzw. des HTTP-Service.

## 3. Abhängigkeiten (Rust-Crates)

- pf-core: serde/serde_json, thiserror, toml, tracing, tracing-subscriber,
  tracing-appender, chrono, uuid, sha2 — klein halten.
- pf-engine: pf-core + serde_json; keine Python-Kenntnis (nur Trait `LlmBridge`).
- pf-bridge: pf-core + pyo3 (Features: `auto-initialize`, `macros`) — Embed-Mode.
  Bewusst KEIN `extension-module`/cdylib in v0.1: `extension-module` und
  `auto-initialize` schließen sich im selben Cargo-Feature-Unifikationsraum
  aus; Python-Tests testen die Python-Schicht direkt, die Rust→Python-Richtung
  wird über den Embed-Mode getestet.
- pf-cli: pf-core, pf-engine, pf-bridge, pf-tui, pf-service, clap, arboard
  (Clipboard), serde_json, anyhow (nur CLI-Glue).
- pf-tui: pf-core, pf-engine, ratatui, crossterm.
- pf-service: pf-core, pf-engine, pf-bridge, axum, tokio, serde, serde_json.

PyO3-Konfiguration: gebaut gegen das uv-Venv (`python/.venv/bin/python`).
Siehe `.cargo/config.toml` (`[env] PYO3_PYTHON`) und `scripts/env.sh`.

## 4. Prompt-IR (pf-core)

Versionierte, JSON-serialisierbare IR (Rust-Structs, serde), bewusst
provider-unabhängig. Felder (Spec §4): `task`, `objective[]`,
`context[]`, `inputs[]`, `constraints[]` (mit `severity`),
`assumptions[]`, `role`, `procedure[]` (geordnet), `reasoning_strategy`,
`examples[]`, `output_contract` (Format, Struktur, Schema, Beispiele),
`verification_requirements[]`, `target_model`, `metadata`
(`schema_version`, `created_at`, `request_id`, Quellsprache, Tags).

Serialisierung: `PromptIr::to_json()/from_json()`; Stabilität über
`schema_version` und Enum-Typen mit `#[serde(tag = "...")]`.

## 5. LLM-Provider-Interface

Rust kennt nur den abstrakten Vertrag (pf-core):

```rust
pub trait LlmBridge: Send + Sync {
    /// Ein LLM-Request (operation + messages + config) → strukturierte Antwort.
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, PfError>;
}
```

- `LlmRequest`: `operation` („architect" | „optimize" | „verify" | „tokenize"),
  `system_prompt`, `messages`, `output_schema` (JSON), `endpoint`, `api_key`
  (nur im Speicher, nie im Log), `model`, `temperature`, `max_tokens`,
  `timeout`, `request_id`.
- `LlmResponse`: `content`, `finish_reason`, `usage` (prompt/completion/total),
  `model`, `duration_ms`.
- Implementierungen: `PythonBridge` (pf-bridge, ruft eingebettetes Python →
  `promptforge.bridge.handle_request`), `MockBridge` (Rust-seitig für
  Engine-Unit-Tests ohne Python), plus Python-seitig `MockLLM` für
  Integrationstests „Rust → PyO3 → Python → Mock-LLM".

Python-seitige Kapselung von any-llm (Spec §2: any-llm dringt nicht in den
Rust-Core): Modul `promptforge/llm/provider_anyllm.py`.

```python
if endpoint:
    client = AnyLLM.create_openai_compatible(name="custom", api_base=endpoint, api_key=key)
else:  # provider:model-Syntax im Modellnamen
    client = AnyLLM.create(provider=provider, api_key=key)
resp = client.completion(model=model, messages=[...])
```

Fehler-Mapping any-llm-Exceptions → pf_error_kind
(authentication/provider/model/timeout/rate_limit…). `provider=mock`
(PF_PROVIDER) aktiviert `MockLLM` ohne Netz.

## 6. Pipeline (pf-engine)

```
intent ─► [architect] ─► PromptIR ─► [expand] ─► LongPrompt ─► [optimize]
        ─► OptimizedPrompt ─► [verify] ─► Ergebnis (oder Re-Optimize, max N)
```

- `architect`: LLM-Aufruf (Python) → validiertes `PromptIr` (Struktur- und
  Pflichtfeld-Validierung in Rust; bei Fehler `OptimizationError`/Retry).
  Ohne LLM (`--no-llm`/Mock): deterministische Basis-IR aus Intent-Text.
- `expand`: deterministische Rust-Template-Expansion aus IR → ausführlicher
  Long Prompt (explizite Rollen, Ziele, Constraints, Output-Contract,
  Verifikationsanforderungen, Annahmen). Lang sein ist hier gewollt.
- `optimize`: Pass-Pipeline, nachvollziehbar über `OptimizerEvent`-Liste:
  1. Analyse-Pass (Token-Zählung je Abschnitt, Duplikat-/Redundanz-Meldungen)
  2. LLM-Pass (Python `optimize`): IR + Long Prompt + Optimierungsrichtlinien
     + Output-Contract → optimierter Prompt (JSON, optional mit Notizen)
  3. Hygiene-Pass (Rust): Whitespace/Zeilen-Kollaps, Nahe-Duplikat-Zeilen
     entfernen, Output-Contract-Marker prüfen (Erhalt erzwingen)
- `verify`: (a) strukturelle Checks in Rust (Objective/Constraints/Output-
  Contract/Vocabular erhalten, Reduktion > 0), (b) optional LLM-Semantik-
  Check (Python `verify`) → strukturiertes `VerificationReport`
  (`semantic_preservation`, `constraints_preserved`, …,
  `verdict: PASS|FAIL|RETRY`).
  Bei `FAIL`/niedriger Fidelity: Re-Optimize mit Feedback; Retry-Limit aus
  Config (`verify.max_attempts`, Default 2), kein Endlos-Loop.
- Jede Pipeline-Stufe loggt Metadaten (request_id, stage, status, token
  counts, duration) — nie Prompt-Inhalte außer opt-in.

## 7. Token-Accounting (pf-core)

Trait `Tokenizer { count(&str) -> u64; name(); is_estimate() }`.
- `HeuristicTokenizer`: Schätzung (Wörter + Interpunktion), `is_estimate=true`.
- `BridgeTokenizer` (optional): exakter Zähler via Python (tiktoken/any-llm),
  wenn für Modell verfügbar; sonst transparenter Fallback mit Kennzeichnung.
- `TokenReport`: original, generated, optimized, reduction %, je Stufe.

## 8. Konfiguration (pf-core)

Reihenfolge: Builtin-Defaults < `$PF_HOME/config/config.toml` < Environment.
- Kernvariablen: `LLM_ENDPOINT`, `LLM_KEY`, `LLM_MODEL` (Spec), dazu
  `PF_HOME`, `PF_LOG_LEVEL`, `PF_LOG_FORMAT` (text|json), `PF_LOG_DIR`,
  `PF_LOG_RETENTION`, `PF_PROMPT_LOG` (0|1, Opt-in Prompt-Logging),
  `PF_PROVIDER` (auto|any_llm|mock), `PF_VERIFY_THRESHOLD`,
  `PF_MAX_ATTEMPTS`, `PF_SERVICE_HOST/PORT`, `PF_COPY`…
- `Config::redacted()` und manuelles Debug: Secrets → `[REDACTED]`.
- Persistenz: nur Prompts/History (gewollt); Keys nur in Env/Config-Datei,
  niemals in Logs. Hinweis OS-Keychain als spätere Option (ADR, nicht v0.1).

## 9. Fehler (pf-core)

`PfError` mit `kind`: Configuration, Provider, Authentication, Model,
Timeout, Tokenization, Optimization, Verification, Persistence, Bridge,
Serialization, Json, Io, InvalidInput. JSON-/CLI-kompatibel (`kind`,
`message`, optional `retryable`). Exit-Codes CLI: 0 ok, 1 generisch, 2 Usage,
3 Config, 4 LLM, 5 Pipeline/Verify, 6 Persistence, 7 Infra/Bridge.

## 10. Persistenz & History (pf-core)

- `~/.prompt-forge/prompt/{templates,generated,optimized,archive}/`
- `~/.prompt-forge/history/` (JSONL-Einträge: request_id, Zeitstempel,
  Intent-Hash, Token-Report, Stufen-Status, Pfade der Artefakte)
- `~/.prompt-forge/state/` (z. B. letzter Request, TUI-State)
- atomares Schreiben (temp + rename), idempotente Dateinamen mit Zeitstempel
  + Kurz-Hash; `PersistenceError` bei Verstößen.

## 11. Logging

- Rust: `tracing` + `tracing-subscriber`/`tracing-appender`, Rolling-Dateien
  `~/.prompt-forge/logs/prompt-forge.{log,N.log}`, Retention konfigurierbar.
- Python: `loguru` mit Interceptor in dasselbe Schema (JSON- oder
  Text-Sink), beide Seiten rotieren bzw. werden konsolidiert.
- Felder: ts, level, target, request_id, stage, model, duration_ms,
  token counts, status. Redaction-Filter auf beiden Seiten.
- `PF_LOG_FORMAT=json` für Service/Machine-Mode.

## 12. CLI (pf-cli, Binary `prompt-forge`)

```
prompt-forge [compile] [TEXT|DATEI]   # stdin, falls nicht tty (auch `-`)
prompt-forge init                     # Verzeichnisse + Default-Config anlegen
prompt-forge serve                    # HTTP-Service (pf-service)
prompt-forge tui                      # TUI (pf-tui)
prompt-forge compile --json …         # maschinenlesbar (Legacy-Alias)
prompt-forge compile --format yaml …  # Envelope als YAML
```

Flags: `-o/--out`, `--copy` (kopiert exakt die erzeugte Serializer-Ausgabe),
`-i/--interactive` (Menü: Copy/Save/Show/Recompile), `--no-llm`,
`--model/--endpoint`-Override, `--format text|json|yaml|toon`, `--json`
(= `--format json`), `--debug` (menschlesbarer Trace auf stderr),
`--debug-json` (Trace als JSON, Datei via `-o`), `-v` (verbose),
`--config` (Config-Pfad). TTY ohne `--json`/strukturiertes Format →
Zusammenfassung + interaktives Menü; Nicht-TTY → Prompt/Envelope auf stdout
(scriptbar), Statistik auf stderr. Exit-Codes nach §9.

**Debug-Trace (`--debug`/`--debug-json`, v0.2)**: Die Engine emittiert pro
LLM-Call ein `StageEvent::LlmTrace` (Stufe, Attempt, System-/User-Prompt als
Echo der Python-Schicht, rohe Antwort — redigiert). CLI baut daraus ein
serialisierbares Dokument `{input, llm_used, stages[]}`; Stufen ohne LLM
sind `llm: false` mit Hinweis, kein künstliches `raw_response`. Bei
Fehlern wird ein partieller Trace geschrieben (kein Verlust der echten
Requests im Fehlerfall).

**Ausgabe-Serialisierung (v0.2, pf-core `serialize`)**:

```text
CompilationResult (formatneutral, kennt keine Formate)
   ├── TextSerializer  → optimized_prompt (ausführbarer Prompt)
   ├── JsonSerializer  → Envelope als JSON (pretty, deterministisch)
   ├── YamlSerializer  → Envelope als YAML (serde_norway 0.9)
   └── ToonSerializer  → Envelope als TOON (toon-format 0.5, Spec v3.0)
```

json/yaml/toon repräsentieren dasselbe Datenmodell (`CompilationResult`-
Envelope inkl. v0.1-Aliasen `ir`/`long_prompt`/`final_output` für
Abwärtskompatibilität); Serialisierung ist deterministisch, rein lokal und
löst keinen LLM-Aufruf aus. `-o` bestimmt das Format über `--format`, nicht
über die Dateiendung.

Clipboard: Trait `Clipboard { set_text }`; Impls: `ArboardClipboard`
(Standard) und Kommando-Fallback (pbcopy/wl-copy/xclip/Set-Clipboard).

## 13. TUI (pf-tui, ratatui)

Anzeige: Eingabebereich, Pipeline-Stadien-Status (Architect ✓, IR, Expansion,
Optimization, Verification), Token-Zahlen, Verifikations-Report,
Prompt-Vorschau (scrollbar), Aktionen: Compile, Copy, Save, Show/Preview,
Recompile, Quit. Pipeline läuft im Worker-Thread, UI pollt Events.

## 14. HTTP-Service (pf-service)

`prompt-forge serve` → axum auf `127.0.0.1:8770` (konfigurierbar).
Endpunkte (Details: docs/api.md):
- `POST /v1/compile`   — Intent → kompilierter Prompt (+ Report); optional
  `format` (text|json|yaml|toon) → `{format, output, input}` (v0.2, additiv)
- `POST /v1/optimize`  — Long Prompt / IR → optimierter Prompt
- `POST /v1/verify`    — Original vs. optimiert → VerificationReport
- `POST /v1/execute`   — fertigen Prompt gegen Ziel-LLM ausführen
- `GET /v1/health`     — Status
Engine = dieselbe wie CLI/TUI. JSON-Fehler-Envelope mit `kind`.

## 15. Sicherheit/Privatsphäre

- Keys/Header/Secrets: Redaction in Logs (beide Seiten), `Debug`-Impls.
- Prompt-Inhalte: nicht in Logs (außer `PF_PROMPT_LOG=1`), Metadaten ja.
- Kein Telefonat nach Hause: Requests nur an konfigurierten Endpunkt.

## 16. Tests

- Rust: Unit-Tests in pf-core (IR-Roundtrip, Config-Präzedenz + Redaction,
  Tokenizer, Optimizer-Passes, Persistenz, Fehler), pf-engine (Pipeline mit
  MockBridge, Verify-Retry), pf-cli (Argument-/Exit-Verhalten).
- Python: pytest für Provider (any-llm-Config, Mock), Response-Normalisierung,
  Fehler-Mapping, Bridge-Schema (handle_request in/out).
- Integration: Rust→PyO3→Python→MockLLM→Python→Rust (Rust-Test in pf-cli,
  env-gesteuert: benötigt `python/.venv`; wird übersprungen mit Hinweis,
  wenn das Venv fehlt); CLI-E2E-Skript mit Mock-Provider; Python-Tests
  testen die Python-Schicht inkl. Bridge-Schema direkt.
- Keine echten Cloud-Keys in Tests; Mock/Fake-LLM überall.

## 17. Bewusst NICHT in v0.1

- GUI (nur Client-Architektur vorbereitet), OS-Keychain-Integration,
  Streaming/parallele LLM-Calls, Plugins, Prompt-Versionierung mit Diff-UI.
