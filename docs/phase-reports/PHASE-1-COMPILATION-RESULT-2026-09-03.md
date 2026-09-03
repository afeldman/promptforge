# PromptForge v0.2 — Phase 1: CompilationResult & Intent Model + Compiler Smoke

Datum: 2026-09-03 · Status: abgeschlossen · Kein Commit / kein Push

---

## STATUS

PASS — Phase 1 vollständig umgesetzt und verifiziert:

- `make verify` (deterministisch: fmt + lint + test + build + Compiler-Smoke): grün
- `make test-apfel` (echter LLM-Pfad, Release-Binary): PASS (semantic 0.90, Reduktion 10.8 % estimated; stochastikbedingt, kein Fake)
- Rust: pf-core **50** Tests, pf-engine **22**, pf-service 3, pf-tui 2, pf-bridge 1, E2E Python-Bridge 1 — 0 failed
- Python: pytest **18 passed / 1 skipped**
- fmt/clippy: 0 Findings

## ARCHITECTURE CHANGES

- **`CompileOutcome` (pf-engine) → `CompilationResult` (pf-core)**: Das v0.1-Ergebnisobjekt wurde nicht dupliziert, sondern in ein formatneutrales, serde-fähiges `pf_core::compilation::CompilationResult` überführt. `Engine::compile()` liefert direkt `CompilationResult` — es gibt weiterhin genau **eine** Pipeline und **ein** Ergebnisobjekt (Anforderung §5: keine zweite Pipeline).
- **Verifikations-Datentypen nach pf-core verschoben** (`pf-core/src/verify.rs`: CheckResult, Verdict, SemanticReport, VerificationReport): nötig, damit `CompilationResult` den Verifikationsbericht referenzieren kann, ohne dass pf-core von pf-engine abhängt (Serializer in Phase 2 leben in pf-core). pf-engine re-exportiert die Typen (`pub use pf_core::verify::…`) — alle bestehenden Importpfade (`pf_engine::Verdict` etc.) bleiben unverändert gültig.
- **Engine berechnet Qualitätsmetriken** (`QualityMetrics::compute(verification, token_report)`): `semantic_fidelity`, `structural_validity`, `token_efficiency` — deterministisch, Phase-1-Minimalumfang (volles Qualitätsmodell folgt Phase 4).
- **JSON-Envelope mit v0.1-Aliassen**: `CompilationResult::envelope_json()` serialisiert kanonische v0.2-Felder (`input`, `prompt_ir`, `expanded_prompt`, `optimized_prompt`, `verification`, `metrics`, …) und fügt die v0.1-Aliase `ir`, `long_prompt`, `final_output` hinzu. CLI `--json`, Service `/v1/compile` und apfel-smoke.sh nutzen diesen Envelope → Abwärtskompatibilität ohne Parallel-Format.
- `format_summary` zeigt zusätzlich die Metrics-Zeile (`semantic · structural · token-efficiency`); History-Einträge enthalten jetzt `intent` (aus `input`).
- TUI und pf-service auf `CompilationResult` umgestellt (nur Feldnamen; UX unverändert).

## PROMPT IR CHANGES

Additiv, ohne Schema-Bruch (schema_version bleibt 1, §3):

- **Neu**: `PromptIr.analysis: Option<IntentAnalysis>` mit `{task_type, profile_hint, language, confidence, notes}` — alle Felder optional, serde-Defaults, `skip_serializing_if` (fehlende Analyse erzeugt kein Feld im JSON).
- **Bewusste Abweichung von der Design-Skizze**: KEIN zusätzliches top-level `tags`. `IrMetadata.tags: Vec<String>` existiert bereits in v0.1 — eine zweite Liste wäre ein Duplikat. Geprüft (Spec §3: „Prüfe genau, welche Struktur im bestehenden IR bereits existiert. Nicht blind die Design-Skizze kopieren.“); im Bericht dokumentiert.
- Alte v0.1-IR-JSONs (ohne `analysis`) deserialisieren identisch (Regressionstest `v01_json_without_analysis_still_deserializes`).

## COMPILATION RESULT

Formatneutral (kein `format`-Feld, keine Serializer-Kenntnis):

```rust
CompilationResult {
    input: String,
    request_id: String,
    llm_used: bool,
    stages: Vec<String>,
    prompt_ir: PromptIr,
    expanded_prompt: String,
    optimized_prompt: String,
    token_report: TokenReport,
    verification: VerificationReport,
    metrics: QualityMetrics,   // semantic_fidelity, structural_validity, token_efficiency
}
```

Serializer (text/json/yaml/toon) kommen in Phase 2 und lesen nur dieses Objekt.

## TESTS

Neu (deterministisch):
- pf-core/ir: `analysis` vorhanden/nicht vorhanden, Roundtrip inkl. Feldwerte, alte v0.1-IR ohne `analysis` deserialisierbar, keine unnötigen Felder bei Serialisierung.
- pf-core/compilation: Construction, Serialisierung/Deserialisierung (Roundtrip), Default-/optional-Felder, `envelope_json` mit Legacy-Aliassen, Metriken positiv/negativ/0 (u. a. `token_efficiency=-0.42`-Fall), `structural_validity` bei false-Boolean.
- pf-core/verify: Typen-Roundtrip, Verdict snake_case/Default, `all_preserved`.
- pf-engine/pipeline: deterministischer E2E-Lauf liefert `CompilationResult` mit `input` (getrimmt), Metriken, 4 Stadien; Mock-LLM-Lauf prüft `prompt_ir.role` + CompilationResult; IR ohne LLM → `analysis=None`.
- pf-cli E2E (Python-Bridge, Mock): prüft CompilationResult-Felder + Envelope-Aliase.

## SMOKE TEST

**Neu**: `tests/compiler-smoke.sh` — deterministischer Compiler-Smoke gegen das **Release-Binary** (echter Codepfad, kein Engine-Mock, kein direktes Instanziieren):

```text
"auditiere das projekt"
   │  prompt-forge compile … --no-llm --json
   ▼
Engine → Prompt IR → Expansion → Optimierung → Verifikation
   ▼
CompilationResult (JSON-Envelope)
   ▼
Python: strukturelle Prüfung (kein Text-Grep)
   ▼
PASS
```

Geprüft maschinenlesbar: exit 0, `input` == Intent, `prompt_ir` mit `task`/`schema_version=1`, `expanded_prompt`/`optimized_prompt` nicht leer, `verification.verdict == "pass"`, `metrics` (structural True, token-efficiency ≥ 0), v0.1-Aliase `ir`/`long_prompt`/`final_output`, Stadien `[architect, expand, optimize, verify]`, `llm_used == false`, `analysis` nicht gesetzt. Zusätzlich deterministischer Text-Modus (`--plain`) stabil.

**Makefile**: neues Target `test-compiler` (abhängig von `build`); in `make test` integriert. `make help` dokumentiert die Targets.

## APFEL INTEGRATION

`tests/providers/apfel/smoke.sh` **erweitert** (kein zweiter paralleler Test): Die Python-Prüfung des JSON-Envelopes verlangt jetzt zusätzlich `input`, `prompt_ir`, `expanded_prompt`, `final_output`, `metrics` (semantic_fidelity/token_efficiency/structural_validity) und die Legacy-Aliase `ir`/`long_prompt`. Ausgabe ergänzt um `[ok] CompilationResult vorhanden …`.

Ergebnis des echten Laufs (Release-Binary, Apple Foundation Model, apfel v1.5.5): **PASS** — Model `apple-foundationmodel`, 25 → 176 → 157 estimated tokens (Reduktion 10,8 %), semantic 0.90, Verdict pass, Server-Lebenszyklus sauber, keine Server-Reste. Keine künstliche Schwellen-Absenkung, kein Fake (FAIL wäre ein legitimes Ergebnis gewesen).

## BACKWARD COMPATIBILITY

| Bereich | Ergebnis |
|---|---|
| v0.1 CLI (`compile`, Default-Text, `--json`, `--copy`, `-o`, stdin/Datei) | erhalten; `--json` liefert zusätzliche Felder + Aliasse |
| v0.1 `--json`-Schlüssel (request_id/llm_used/stages/ir/long_prompt/optimized_prompt/token_report/verification) | alle weiterhin vorhanden (Aliase) |
| v0.1 deterministischer Compile | unverändert, jetzt mit `metrics` |
| v0.1 Python-Bridge-Vertrag | unverändert (keine Python-Änderung nötig) |
| v0.1 Service-Endpunkte | /v1/compile-Response additiv erweitert; übrige Endpunkte unverändert |
| v0.1 Prompt IR / JSON-Dateien | `analysis` optional → alte Dateien parsen identisch |
| v0.1 apfel smoke | erweitert um v0.2-Checks; alte Checks unverändert |
| v0.1 Rust-Tests | alle grün (pf-core 37-Bestand enthalten, Zählung jetzt 50) |

**Explizite Makefile-Änderung (mit Begründung)**: `make verify` ist jetzt deterministisch und enthält **keinen** apfel-Teil mehr; der echte LLM-Pfad läuft separat über `make test-apfel` bzw. neu `make verify-all`. Begründung: Der Apple-Foundation-Model-E2E ist modellstochastisch (degenerierte Optimize-Ausgaben trotz grüner Gates, bereits in v0.1 dokumentiert); ein reproduzierbares CI-/Entwicklungs-Gate darf davon nicht abhängen. Die Änderung ist im Makefile als Design-Kommentar dokumentiert (§12 der Aufgabe verlangte Begründung im Bericht). `make test` bleibt vollständig deterministisch (Rust + Python + Compiler-Smoke).

## FILES CHANGED

```text
crates/pf-core/src/ir.rs              analysis-Feld + IntentAnalysis + Tests (+3)
crates/pf-core/src/verify.rs          NEU: Verifikations-Datentypen (aus pf-engine) + Tests (+3)
crates/pf-core/src/compilation.rs     NEU: CompilationResult + QualityMetrics + Tests (+6)
crates/pf-core/src/lib.rs             Module + Re-Exports
crates/pf-engine/src/verify.rs        Typen entfernt → Re-Export aus pf-core
crates/pf-engine/src/pipeline.rs      CompileOutcome → CompilationResult; Metriken; Tests (+1)
crates/pf-engine/src/lib.rs           Re-Exports, compile_deterministic-Typ
crates/pf-cli/src/app.rs              CompilationResult; Envelope; History-Intent; Metrics in Summary
crates/pf-cli/src/main.rs             CompilationResult; History-Intent
crates/pf-cli/tests/e2e_python_bridge.rs  Feldnamen + CompilationResult-Asserts
crates/pf-tui/src/app.rs              CompilationResult (UiMsg/current_outcome/save)
crates/pf-service/src/lib.rs          /v1/compile liefert envelope_json()
tests/compiler-smoke.sh               NEU: deterministischer Compiler-Smoke (Release-Binary)
tests/providers/apfel/smoke.sh        v0.2-CompilationResult-Checks ergänzt
Makefile                              test-compiler; verify deterministisch; verify-all; help
README.md                             Schnellstart-Abschnitt aktualisiert
```

## GIT STATUS

```text
?? docs/phase-reports/DESIGN-v0.2-PROMPT-COMPILER-2026-09-03.md   (aus Design-Phase)
?? docs/phase-reports/PHASE-1-COMPILATION-RESULT-2026-09-03.md    (dieser Bericht)
?? tests/compiler-smoke.sh
 M Makefile
 M README.md
 M crates/pf-cli/src/app.rs
 M crates/pf-cli/src/main.rs
 M crates/pf-cli/tests/e2e_python_bridge.rs
 M crates/pf-core/src/compilation.rs
 M crates/pf-core/src/ir.rs
 M crates/pf-core/src/lib.rs
 M crates/pf-core/src/verify.rs
 M crates/pf-engine/src/lib.rs
 M crates/pf-engine/src/pipeline.rs
 M crates/pf-engine/src/verify.rs
 M crates/pf-service/src/lib.rs
 M crates/pf-tui/src/app.rs
 M tests/providers/apfel/smoke.sh
```
Kein Commit, kein Push; keine Änderungen an Commits; `~/.prompt-forge` unangetastet (Smoke nutzt `mktemp`-Homes); keine Secrets/Logs/Tempdaten im Repo.

## KNOWN LIMITATIONS

- `CompilationResult`/`QualityMetrics` sind bewusst Phase-1-Minimalumfang: Das vollständige Qualitätsmodell (instruction/output_contract quality, verdict DEGRADED, Gates) folgt in Phase 4; Serializer (text/json/yaml/toon, `--format`) in Phase 2.
- `envelope_json` dupliziert die IR als `prompt_ir` + `ir` (Übergangskompatibilität) — in Phase 2/3 kann der Legacy-Alias entfallen.
- apfel-E2E bleibt stochastisch; `make verify` ist davon entkoppelt (dokumentiert).
- Intent-Analyse-Felder (`analysis`) werden von der Engine noch nicht befüllt (LLM-Intent-Schicht folgt Phase 3) — Struktur und Vertrag sind vorbereitet.

## NEXT STEP

Phase 2: Serializer-Schicht (text/json/yaml/toon) + `--format`/`--format json`-CLI; danach Phase 3 (Intent Analysis mit `analysis`-Feldern durch den Prompt Generator).
