# PromptForge — Repair CI + Apfel Robustness (Phase Report)

Datum: 2026-09-03 · Repo: PromptForge · Kein Commit, kein Push.
Basis: HEAD b9873ba (extern, inkl. v0.2/v1.0-Arbeiten) + uncommitted Repair.

## 1. Ausgangsproblem

1. GitHub Actions CI „crasht“ (Bericht ohne lokalen GitHub-Log).
2. Echter apfel-Pfad (Apple Foundation Model) scheitert sporadisch mit
   `Fehler (4): model: Python: Architect lieferte kein JSON` — die Antwort
   beginnt mit gültigem JSON (`{"schema_version": 1, "task": …`), ist aber
   abgeschnitten; Meldung war generisch und nicht diagnostisch.

## 2. Root Cause CI

Kein GitHub-Log lokal verfügbar → **GITHUB EXECUTION: NOT VERIFIED** (ehrlich;
siehe §17 Abschlussbericht). Workflow-Audit ergab strukturelle Fehler, die
einen sauberen Lauf verhindern bzw. den apfel-Job wertlos machen:

1. **apfel-Job baute das Release-Binary nie** — `smoke.sh` endet sofort mit
   „release binary not found“; `continue-on-error: true` verbarg das → der
   Job testete faktisch nichts.
2. **Impliziter, cwd-abhängiger `PYO3_PYTHON`-Pfad** (`python/.venv/bin/python`
   relativ; je nach `working-directory`/cd inkonsistent).
3. **Kein explizites Python-Setup** über `setup-python`: Python kommt implizit
   aus uv-managed Downloads; für PyO3 ist der exakte Interpreter kritisch.
4. Step-lokale `env:`-Overrides (relativ) konnten den Job-Env überschreiben.

## 3. Root Cause Apfel (Architect-Truncation)

- `max_tokens` wurde **nie gesendet** (Config-Default `None`, `LLM_MAX_TOKENS`
  nicht gesetzt): Apple Foundation Model kappt Antworten ohne max_tokens am
  Server-Default-Output-Limit mitten im JSON.
- Der Architect-Prompt lud zu langen, redundanten Listeneinträgen ein
  (keine Längen-/Abschlussregeln).
- Der Parser meldete jeden JSON-Fehler generisch („lieferte kein JSON“) —
  ohne Unterscheidung empty/invalid/truncated/schema und ohne
  finish_reason=length-Auswertung.
- Messung: direkter `/v1/chat/completions`-Call mit `max_tokens=3072` liefert
  vollständige IRs (`finish_reason=stop`, 683 completion tokens, 2539 Zeichen);
  ohne explizites Limit bricht das Modell je nach Ausgabelänge ab.
  Zusätzlich tritt reine Modellvarianz auf (z. B. `Expecting ',' delimiter`
  mitten im JSON) — reparabel durch einen zweiten echten Request.

## 4. Änderungen

### CI (` .github/workflows/ci.yml`)
- `PYO3_PYTHON` global auf **absoluten Pfad** `${{ github.workspace }}/python/.venv/bin/python` (kein impliziter relativer Pfad).
- Step-lokale relative `env`-Overrides entfernt (nur Job-Env).
- apfel-Job: **`cargo build --release` vor `smoke.sh`** (echter Test des
  Release-Binary); `continue-on-error` bleibt NUR auf dem optionalen
  apfel-Schritt — der deterministische `ci`-Job bleibt ein echtes Blocking
  Gate (fmt/clippy/test/pytest/build/compiler-smoke; identisch mit
  `make verify`).
- YAML lokal validiert (Ruby-Parser): gültig, jobs = ci + apfel-integration.

### Architect-Response-Contract
- `prompts.py ARCHITECT_SYSTEM`: Längenregeln (Listeneinträge < ~12 Wörter,
  keine Erklärungen/Epiloge, JSON endet exakt mit `}`), Vollständigkeit
  (kein Feld weglassen, Einträge nicht zusammenfassen). Keine Felder wurden
  entfernt — nur Output-Länge/Disziplin adressiert.
- `bridge.py`: neue Klassifikation `_json_parse_failure()`:
  `empty response | invalid JSON | truncated JSON (…appears truncated before
  valid JSON completion, ggf. finish_reason=length) | schema violation`.
  Head+Tail-Snippets bleiben in der Meldung; vollständiger Rohtext bei Erfolg
  im `--debug-json`-Trace.
- `pipeline.rs`: **begrenzter Architect-Retry** (max. 1 zusätzlicher Request,
  attempt 2 im Trace, Note sichtbar) NUR bei reparablen Fehlern
  (invalid JSON / schema violation). **Kein Retry bei Truncation/empty** —
  Ursache ist das Output-Limit; ein identischer zweiter Call würde wieder
  kappen (kein Fake-PASS durch Wiederholen).

### Output-Limit
- `tests/providers/apfel/smoke.sh`: `LLM_MAX_TOKENS="${LLM_MAX_TOKENS:-3072}"`
  im echten Pfad (3072 < 4096-Kontext).
- README: `LLM_MAX_TOKENS` dokumentiert (Empfehlung bei lokalen Modellen).

## 5. Tests (neu/erweitert)

- Python `test_bridge_mock.py` +8:
  valid JSON→IR, truncated JSON diagnostiziert, truncated + finish_reason=length,
  empty response, schema violation, Verify truncated, Echo von System/User-Prompt
  im Erfolgsfall (echter Rust→Python→Provider→Python-Vertrag; nur Chat gepatched).
- Rust `pf-engine` +2 (`pipeline.rs`): `architect_invalid_json_is_retried_once_and_recovers`
  (Flaky-Bridge: 1. Call invalid → 2. Call valide → compile PASS),
  `architect_truncation_is_not_retried` (Fehler kind=model, Meldung enthält
  „truncated“, kein zweiter Call).
- pf-engine gesamt: 36 lib-Tests grün; pytest: 26 Tests (18 alt + 8 neu) —
  siehe Abschlussgates.

## 6. Acceptance Test (real, apfel)

`make apfel-start` → `compile "auditiere das projekt" --debug-json`:

- Exit 0; `optimization_status=optimized`, `selected=redundancy` (echter
  Engine-Lauf); Verifikation bestanden.
- Trace enthält echte Werte: architect attempt raw_len=1402, optimize 1798,
  verify 202 (system/user/raw — redigiert), expand `llm=false`.
- Retry real beobachtet: Note
  „Architect-Parsing-Fehler (… invalid JSON (Expecting ',' delimiter …):
  ein erneuter Versuch“ → zweiter Call erfolgreich.
- Keine Secrets in Trace/Logs.

## 7. CI-Verifikation

- Lokal: `make verify` PASS (fmt, clippy -D warnings, cargo test --workspace,
  pytest, release build, compiler-smoke) — gleiche Schritte wie CI-Job.
- Workflow-YAML syntaktisch gültig; Actions gepinnt/Versionen geprüft
  (checkout@v4, setup-uv@v3, dtolnay/rust-toolchain@master [Rust 1.98],
  upload-artifact@v4). Keine Secrets im Workflow.
- **GITHUB EXECUTION: NOT VERIFIED** (kein GitHub-Zugriff in dieser Session).

## 8. Apfel-Verifikation

Siehe §6; komplette `make test-apfel`-Suite-Ergebnis im Abschlussgates-Block
unten ergänzt (separater Lauf, stochastisch).

## 9. Security-Verifikation

- Redaction unverändert aktiv (Rust + Python); LLM_KEY-Werte nur als gesetzt
  behandelt, nie ausgegeben.
- Neue Fehlermeldungen enthalten nur Head/Tail-Snippets der Modellantworten
  (keine Secrets; Modellantworten sind ohnehin Prompt-Texte — redigiert via
  sanitize_line wenn sie durch den LlmTrace laufen).
- Keine echten Secrets in Tests/Artefakten.

## 10. Bekannte Limitierungen

1. `make test-apfel` bleibt stochastisch (Modellvarianz): auch nach Fix können
   einzelne Architect-Antworten zweimal fehlerhaft sein → ehrlicher exit 4.
2. Truncation wird diagnostisch erkannt, aber nicht repariert (bewusst); die
   Heilung ist das explizite Output-Limit (LLM_MAX_TOKENS) + kürzere IR-Werte.
3. Vollständiger Rohtext fehlgeschlagener Antworten ist nur in den
   Meldungs-Snippets verfügbar (Erfolgs-Traces enthalten den vollen Rohtext).
4. dtolnay/rust-toolchain@master ist ungepinnt (übliche Praxis; alternativ
   commit-pinnen).
5. GitHub-Runner: apfel-Job auf macos-latest; falls der Runner x64 ist,
   skippt smoke.sh selbst (Apple-Silicon-Check) — Job bleibt dann no-op.

## 11. Verbleibende Risiken

- GitHub-Lauf nicht lokal verifizierbar (Risiko: Umgebungsdetails von
  ubuntu-latest/macos-latest, uv-managed Python vs. PyO3).
- Modellqualität (kleines On-Device-Modell) bleibt der größte Varianzfaktor
  im echten LLM-Pfad.

## 12. Git Status

Kein Commit/Push. Uncommitted (Repair):
- `.github/workflows/ci.yml` (geändert)
- `python/src/promptforge/bridge.py`, `prompts.py`, `python/tests/test_bridge_mock.py`
- `crates/pf-engine/src/pipeline.rs` (Retry + Tests)
- `tests/providers/apfel/smoke.sh`, `README.md`, `docs/api.md`,
  `docs/architecture.md`, dieser Bericht
- zzgl. v1.0-Arbeiten aus vorheriger Phase (Modifikationen/untracked gemäß
  git status im Abschlussbericht).

## 13. Abschlussgates (dieser Stand)

- make verify: PASS
- make optimizer-test: PASS
- make optimizer-benchmark: PASS (Referenzwerte bestätigt: intent1 structural
  −29,9 %, intent2 −26,0 %, intent3 −19,4 %, intent4 −12,3 %)
- make test-apfel: **PASS** (komplett; realer apfel-Lauf 2026-09-03):
  Hauptsuite PASS (25→628→559 estimated, −11,0 %, semantic 1.00);
  Optimization-Benchmark über 3 Intents PASS:
  INTENT 1 selected=redundancy, semantic 0.95, technical 1.00
  INTENT 2 selected=redundancy, semantic 0.90, technical 1.00
  INTENT 3 selected=redundancy, semantic 0.99, technical 1.00
  (INTENT 2 — der zuvor truncation-failende englische Audit-Intent — lief
  nach Fix vollständig durch; LLM_MAX_TOKENS=3072 aktiv, Retry sichtbar.)
- apfel-stop: ausgeführt (pid 14713 beendet), **keine apfel-Prozess-Reste**;
  pytest: 25 passed / 1 skipped.
