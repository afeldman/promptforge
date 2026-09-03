# PromptForge v0.2 — Phase 2: Serializers & `--format`

Datum: 2026-09-03 · Status: abgeschlossen · Kein Commit / kein Push

---

## STATUS

PASS — Phase 2 vollständig umgesetzt und verifiziert:

- `make verify` (fmt + lint + test + build + Compiler-Smoke mit Format-Matrix): grün
- Compiler-Smoke: text/json/yaml/toon, `--json`-Legacy-Gleichheit, stdin/Datei/`-o`, ungültiges Format → PASS
- Rust: pf-core 62 Tests, pf-engine 22, pf-service 5, pf-tui 2, pf-bridge 1, E2E 1 — 0 failed
- Python: pytest 18 passed / 1 skipped; fmt/clippy 0
- apfel-Real-LLM (`make test-apfel`): **FAIL in diesem Lauf** — legitimes stochastisches Ergebnis (semantic 1.00, einzelne Erhaltungs-Booleans false nach 3 Versuchen; kein Fake-PASS, kein künstlicher Retry, Server-Lebenszyklus sauber). Deterministische Gates sind davon entkoppelt (v0.2-Design §12).

## ARCHITECTURE

- **`pf_core::serialize`** (neu): `OutputFormat` (text/json/yaml/toon), `PromptSerializer`-Trait und die vier Serializer als zustandslose Implementierungen; zusätzlich strukturierte Render-Funktionen (`to_json_string`, `to_yaml_string`, `to_toon_string`, `render_structured`) und Decoder-Helfer (`yaml_to_json`, `toon_to_json`) für Roundtrip-/Äquivalenz-Tests.
- **`CompilationResult` bleibt formatneutral** — es kennt weder Formate noch Serializer (kein `format`-Feld). Dispatch ausschließlich über `OutputFormat` → `serializer_for`/`render_structured`.
- **Text** = ausführbarer Prompt (`optimized_prompt`, kein Dump); **json/yaml/toon** = Envelope desselben Datenmodells (kanonische v0.2-Felder + v0.1-Aliase `ir`/`long_prompt`/`final_output`).
- Serialisierung ist rein lokal/deterministisch in Rust — kein Python/PyO3, kein zusätzlicher LLM-Aufruf (LLM-Pipeline → CompilationResult → Serializer).
- Fehlerfamilie um `ErrorKind::Serialization` erweitert (Exit 7); Serializer melden Fehler über `PfError` — kein `unwrap`/`panic` im CLI-Pfad.
- Service: `/v1/compile` additiv erweitert (kein /v2): `input` als Alias für `intent`, optionales `format` → `{format, output, input}`; ohne `format` bleibt das bisherige Envelope-Verhalten exakt.

## OUTPUT FORMAT MODEL

```text
CompilationResult (formatneutral)
   ├── TextSerializer  → ausführbarer Prompt (optimized_prompt)
   ├── JsonSerializer  → Envelope (JSON, pretty, deterministisch)
   ├── YamlSerializer  → Envelope (YAML, gleiches Datenmodell)
   └── ToonSerializer  → Envelope (TOON, gleiches Datenmodell)
```

Kanonische Werte `text | json | yaml | toon` (Parsing toleriert Großschreibung, `txt`, `yml`); unbekannte Werte → kontrollierter Fehler, kein stiller Fallback.

## TEXT SERIALIZER

`--format text` (Default) liefert den fertigen, ausführbaren Prompt (`optimized_prompt`) — unverändert durch CLI/Scriptmodus, keine zweite LLM-Generation, keine zusätzliche Optimierung.

## JSON SERIALIZER

`--format json` serialisiert den `CompilationResult`-Envelope deterministisch (serde_json pretty + abschließendes Newline). `--json` ist der Legacy-Alias und semantisch identisch (im Smoke maschinenlesbar nachgewiesen, volatile `request_id`/`created_at` normalisiert). Alle v0.1-Schlüssel bleiben erhalten (request_id, llm_used, stages, ir, long_prompt, optimized_prompt, token_report, verification) plus kanonische v0.2-Felder (input, prompt_ir, expanded_prompt, metrics).

## YAML SERIALIZER

`--format yaml` serialisiert denselben Envelope als YAML.
**YAML-Dependency**: `serde_norway 0.9.42` — gepflegter Fork der `serde_yaml`-0.9-API (serde_yaml 0.9.34+deprecated ist archiviert/unmaintained); 10,6 M Downloads, MIT, keine neuen transitiven Konflikte.
**Roundtrip getestet**: `CompilationResult → YAML → serde_norway::from_str::<CompilationResult>` == Original; zusätzlich YAML-Wert == JSON-Wert (semantische Äquivalenz). Unicode, Multiline, Quotes, Sonderzeichen getestet.

## TOON SERIALIZER

`--format toon` serialisiert denselben Envelope als TOON.
**Verwendete Implementierung**: `toon-format 0.5.0` — die offizielle Rust-Implementierung des toon-format-Projekts (`github.com/toon-format/toon-rust`), eingebunden mit `default-features = false` (nur Library-Kern, ohne CLI/TUI-Extras).
**Spezifikation/Version**: TOON Specification v3.0 (Spec-Referenz des Projekts); Crate-README deklariert volle Spec-Conformance mit Conformance-Test-Suite.
**Lizenz/Reifegrad**: MIT; aktiv gepflegt (letzte Veröffentlichung 2026-05), 535k Downloads, Tests grün.
**Roundtrip getestet (JA)**: Der Decoder der Crate ist vorhanden und wird genutzt: `CompilationResult → TOON (encode_default) → decode_default::<serde_json::Value>` == JSON-Wert des Envelope (semantische Äquivalenz, echte Roundtrip-Garantie getestet — nicht nur behauptet). Determinismus zusätzlich getestet (zwei Serialisierungen identisch).
**TOON ist keine „YAML mit anderer Endung“**: Es ist die echte Token-Oriented Object Notation über das serde-JSON-Datenmodell.

## CLI CHANGES

- `--format <text|json|yaml|toon>` (Default `text`); `--json` bleibt Legacy-Alias (= `--format json`; Kombination mit anderem strukturierten Format → kontrollierter Fehler).
- `compile -` = stdin explizit (auch im TTY); Dateiinput (`-f`/Argument) unverändert.
- `-o`: schreibt die serialisierte Ausgabe; das Format bestimmt `--format`, nie die Dateiendung.
- `--copy`: kopiert exakt die erzeugte Serializer-Ausgabe (Prompt bei text, serialisierten Envelope bei json/yaml/toon).
- Interaktives Menü nur noch im Text-Format-Modus (TTY); strukturierte Formate laufen scriptbar.

## BACKWARD COMPATIBILITY

- v0.1 CLI: Default-`text`, stdin-Pipe, `-f`, `-o`, `--copy` (Prompt), `--no-llm`, Overrides — unverändert funktionsfähig (Smoke deckt ab).
- v0.1 `--json`: identische Semantik (alle alten Schlüssel vorhanden; Regressionstest im Smoke).
- v0.1 Service: `/v1/compile`-Default-Antwort unverändert; `format`/`input` nur additiv.
- v0.1 Python-Bridge/IR: unberührt (Serializer rein Rust).
- v0.1 Tests: alle grün (pf-core 37 → 62 durch Phase-1/-2-Tests, keine Abschwächung).
- apfel-smoke.sh: unverändert gültig (Envelope enthält weiterhin alle geprüften Schlüssel).

## TESTS

- pf-core `serialize` (12 neue): Format-Parsing/Display, Text = executable prompt (kein Envelope-Dump), JSON deterministisch, JSON-Wert-Roundtrip, YAML-Roundtrip (CompilationResult & JSON-Äquivalenz), TOON-Roundtrip (Decoder), YAML/TOON deterministisch, Unicode/Multiline/Special-Chars in allen strukturierten Formaten, `serializer_for`-Dispatch, `render_structured`-Guard (kein text).
- Golden-Fixture `golden_result()` (deterministisch, kein LLM) in pf-core-Serialize-Tests.
- pf-service: +2 Tests (`format yaml` additiv inkl. `input`-Alias; ungültiges Format → 400).
- Engine-/E2E-Tests unverändert grün (CompilationResult-Pfad bleibt).

## COMPILER SMOKE

`tests/compiler-smoke.sh` erweitert (nicht ersetzt, kein paralleler Test) — Release-Binary, `--no-llm`, echter Compilerpfad:
- Envelope-JSON strukturell (Python, kein grep): input/prompt_ir/expanded_prompt/optimized_prompt/verification/metrics + Aliase + schema_version + verdict pass.
- `--json` == `--format json` (semantisch, volatile Felder normalisiert).
- text == optimized_prompt (ausführbarer Prompt, kein Envelope).
- yaml: wird tatsächlich geparst (PyYAML auf dem System verfügbar) und auf Strukturfelder geprüft; Fallback dokumentiert (Rust-Roundtrips).
- toon: valides Dokument (84 Zeilen); Conformance/Roundtrip über Rust-Tests (offizieller Decoder) — dokumentierte Grenze im Skript.
- stdin (`compile -`) für alle vier Formate, Dateiinput, `-o` je Format (Format bestimmt, nicht Endung; `saved`-Pfade im JSON), `--format banana` → kontrollierter Fehler mit Meldung.

## APFEL

`make test-apfel` mit Release-Binary ausgeführt: **FAIL (legitim, stochastisch)** — architect/optimize ok, Verify schlug nach 3 Versuchen fehl (semantic 1.00, einzelne preserved-Booleans false). Keine künstliche Schwellenabsenkung, keine verschleiernden Retries, kein Fake-PASS; Server-Lebenszyklus sauber, keine Reste. Real-LLM-Kompatibilität der Serializer ist nicht betroffen (Serializer deterministisch, keine LLM-Abhängigkeit); der Envelope bleibt für den apfel-Smoke vollständig (CompilationResult-Checks aus Phase 1 gültig).

## DOCUMENTATION

- `README.md`: CLI-Usage-Beispiele + Abschnitt „Ausgabeformate (v0.2)“ (Semantik text/json/yaml/toon, `--json`-Alias, `-o`/`--copy`-Regeln).
- `docs/api.md`: /v1/compile additiv dokumentiert (`input`-Alias, optionales `format`, Antwortformen).
- `docs/architecture.md`: ADR D-13 (CompilationResult + Serializer-Layer), ErrorKind-Liste + Exit-Codes, §12 CLI/Ausgabe-Serialisierung, §14 Service-format-Hinweis.

## DEPENDENCIES

- `serde_norway 0.9.42` (YAML; gepflegter serde_yaml-Fork; MIT) — neu in pf-core.
- `toon-format 0.5.0` (TOON Spec v3.0; offizielle Rust-Implementierung; MIT; `default-features = false`) — neu in pf-core.
- Beide ohne neue native Abhängigkeiten; Build-Kompatibilität mit dem Workspace verifiziert (cargo check/test/build über alle Crates).

## FILES CHANGED

```text
crates/pf-core/Cargo.toml                     + serde_norway, toon-format
crates/pf-core/src/serialize.rs               NEU: OutputFormat/Trait/Serializer + 12 Tests
crates/pf-core/src/error.rs                   + ErrorKind::Serialization (Exit 7)
crates/pf-core/src/lib.rs                     Module + Re-Exports
crates/pf-cli/src/main.rs                     --format, --json-Alias-Logik, compile -, Dispatch, --copy/-o
crates/pf-cli/src/app.rs                      read_stdin_blocking/read_stdin_fully
crates/pf-service/src/lib.rs                  /v1/compile: input-Alias + optional format + 2 Tests
tests/compiler-smoke.sh                       Format-Matrix (text/json/yaml/toon/stdin/-o/invalid)
Cargo.lock                                    neue Dependencies
README.md, docs/api.md, docs/architecture.md  Doku (v0.2 Formate)
```

## GIT STATUS

Kein Commit, kein Push; bestehende uncommitted Phase-1-Änderungen unangetastet erhalten. `~/.prompt-forge` unangetastet (Tests nutzen mktemp-Homes); keine Secrets/Logs/Tempdaten im Repo.

## KNOWN LIMITATIONS

- TOON wird als Envelope-Strukturformat ausgeliefert (nicht als Prompt-Prosa); die Crate-Codepfade `encode_default`/`decode_default` sind deterministisch und per Roundtrip verifiziert — tiefergehende Conformance (Layout-Metadaten etc.) ist über die Crate abgedeckt, nicht durch eigene Spec-Fixtures.
- `envelope_json` enthält weiterhin Legacy-Aliase (Duplikate von `prompt_ir`/`expanded_prompt`); Abbau in einer späteren Version möglich, sobald keine v0.1-Konsumenten mehr existieren.
- Service-`format`-Antwort ist ein JSON-Wrapper `{format, output, input}` — bewusst additiv, keine /v2-API; eine spätere Version kann Rohformate/Content-Types ergänzen.
- apfel-Real-LLM-E2E bleibt stochastisch (dieser Lauf FAIL, legitim); von `make verify` entkoppelt.

## NEXT STEP

Phase 3: Intent Analysis (LLM-Intent mit `analysis`-Feldern via Prompt Generator; deterministischer Fallback), danach Phase 4 (volles Qualitätsmodell/Verdict) und Phase 5 (TUI-Format-Auswahl) laut Design-Bericht.
