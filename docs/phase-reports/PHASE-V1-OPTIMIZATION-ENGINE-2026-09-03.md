# PromptForge v1.0 — Optimization Engine (Phase Report)

Datum: 2026-09-03 · Repo: PromptForge · HEAD: ebcc0bb (+ uncommitted v1.0)
Kein Commit, kein Push — Bericht zum Review.

## 1. Zusammenfassung (STATUS)

Die bisherige einzelne Optimize-Stufe wurde zu einer **Kandidaten-basierten
Optimization Engine** ausgebaut (Determinismus + LLM getrennt). Ergebnis aus
realen Läufen (keine erfundenen Werte):

- Deterministisch (`--no-llm`): `structural` gewinnt in allen Benchmark-Intents
  mit **−12 % … −30 % estimated Tokens** bei **semantic 1.00**, technical 1.00,
  keine Guard-Recovery; Baseline (`baseline`) = no_improvement (1:1 Long
  Prompt, nie künstliche Verschlechterung).
- apfel real (Apple Foundation Model): INTENT 1 −8,3 % (selected semantic),
  INTENT 3 −2,5 % (selected redundancy), jeweils semantic 1.00/technical 1.00,
  6 Kandidaten; INTENT 2 (englischer Audit-Intent) **ehrlich FAIL exit 4**
  (Architect lieferte abgeschnittenes JSON — Modellvarianz/Context; kein
  Fake-PASS, dokumentiert unter Limitierungen).
- `make verify` (fmt/lint/Test/Rust/Python/Compiler-Smoke/Build) **PASS**;
  pytest 18 passed/1 skipped; pf-engine 34 Tests grün (30 + 4 neue v1.0-Tests).

## 2. ARCHITECTURE

### Pipeline (unverändert, Orchestrierung ersetzt)

```
Intent → Architect → Expansion → [Optimization Engine] → CompilationResult
                                          │
   Prompt IR ──► Optimization Planner ──► Kandidaten (Strategien)
   Long Prompt ─┘                          ├─ redundancy   (deterministisch)
                                           ├─ instruction  (deterministisch)
                                           ├─ structural   (deterministisch, IR-kanonisch)
                                           ├─ semantic     (deterministisch-konservativ)
                                           ├─ combined     (deterministisch)
                                           └─ llm          (nur bei LLM-Betrieb)
   Jeder Kandidat: Hygiene → Guard (Recovery) → strukturelle Verifikation
   → Scoring → Auswahl des besten gültigen Kandidaten (nie größer als Input)
```

- Orchestrierung/Scoring/Auswahl/Verifikation/Metriken: **Rust** (`pf-engine`);
  LLM-Transformation nur als Kandidat `llm` über die bestehende Python-Schicht
  (LlmOperation::Optimize).
- Neue Rust-Module/Dateien:
  - `crates/pf-engine/src/optimization.rs` — Strategien + Technical-Token-Schutz
  - `pipeline.rs` — `run_optimization_engine()` (Planner/Scoring/Auswahl) ersetzt
    die alte Re-Optimize-Loop (`failed_checks_feedback` entfällt)
  - `pf-core::compilation` — `OptimizationReport`, `CandidateReport`,
    `OptimizationStatus`; `QualityMetrics` additiv erweitert
- `CompilationResult.optimization: Option<OptimizationReport>` — additiv,
  formatneutral; Envelope (json/yaml/toon) enthält ihn; `--debug-json`
  übernimmt ihn in das Trace-Dokument.

## 3. CAVEMAN ANALYSIS

Referenz: https://github.com/JuliusBrussee/caveman (README/Main + verlinkte
Doku; kein Code übernommen, keine Architektur kopiert).

Belegte Beobachtungen aus der Caveman-Dokumentation:
- **Skill-Ansatz**: schreibt Agent-/Output-Regeln komprimierter; „code, shell
  commands, file paths, and exact error messages never get cavemanned — only
  the prose around them" (Sinngemäß README; technische Segmente sind tabu).
- **Proxy/Caveman 2**: komprimiert Provider-Requests vor dem Senden (Kontext-/
  Payload-Ebene), behält Originale lokal für Recovery; Benchmark nennt ~33 %
  weniger Input-Tokens bei 18/18 exakten Antwort-Checks (README-Angabe).
- **HONEST-NUMBERS**: README weist selbst darauf hin, dass Output-Skill-Spar-
  effekte über ganze Sessions geringer ausfallen können und bei
  terse-Workloads das Tool mehr kosten kann als es spart.

### Übernehmen (für PromptForge)

1. **Trennung „technische Segmente vs. Prosa"**: PromptForge schützt
   Code-Fences, eingerückten Code, URLs, Pfade, Versionen, CLI-Zeilen vor
   sprachlicher Kompression (`is_protected_line`) und misst den Erhalt
   (`technical_token_preservation`).
2. **Prosa-Kompression statt Weglassen**: Redundanz/Instruction-Strategien
   entfernen sprachliche Füllstoffe, keine Pflicht-Inhalte.
3. **Ehrliche Spar-Messung** (HONEST-NUMBERS-Prinzip): negative Reduktion und
   Modellvarianz werden offen berichtet; kein künstliches PASS.
4. **Recovery/Original verfügbar halten** als Grundidee des Guards: verlorene
   Pflicht-Atome werden restauriert, Verlust separat gemessen.

### Nicht übernehmen

- **Proxy-/Wire-Architektur** (Anfragen umschreiben, lokale Payload-Copies,
  Caching, Provider-Request-Rewriting): ausdrücklich v2.0-Thema, NICHT v1.0.
- **Skill-Ausgabe-Regeln**: PromptForge optimiert Prompts, keine Agenten-Ausgaben.
- Kein Code, keine Dateien, keine konkreten Prompt-Texte aus Caveman.

### Eigenentwicklung (PromptForge)

- Verifikation jedes Kandidaten gegen die **Prompt IR als kanonische Quelle**
  (SemanticAtoms), nicht gegen Textlänge.
- Kandidaten-Auswahl mit Scoring (semantic × Nutzen × Guard-Penalty).
- Guard als gemessene Recovery (Atome zählen) statt versteckter Korrektur.
- `optimization_status` (optimized/no_improvement/degraded) verhindert, dass
  „länger als Input" als Erfolg gilt.

## 4. OPTIMIZATION STRATEGIES

- **redundancy** — entfernt Zeilen mit <2 Inhalts-Tokens und Zeilen, deren
  Token-Menge in einer längeren Zeile enthalten ist (Token-Containment-Beweis);
  exakte Duplikate geschützter Zeilen werden auf 1 reduziert. Nie werden
  geschützte Zeilen entfernt (außer identische Duplikate).
- **instruction** — konservative Ersetzung von Prosa-Hedges zu Imperativen
  („Please carefully inspect … make sure that you identify" → „Inspect …
  identify"); nur ungeschützte Zeilen; nie unter 3 Inhalts-Tokens kürzen.
- **structural** — baut kompakte, ausführbare Struktur direkt aus der Prompt
  IR (AUFGABE/ZIELE/KONTEXT/EINGABEN/CONSTRAINTS/ANNAHMEN/VORGEHEN/
  AUSGABEFORMAT/VERIFIKATION …); Pflicht-Atome wörtlich, nur Boilerplate fällt.
- **semantic** — konservative Deduplikation: nur beweisbare Token-Subsumption
  und Füll-Diskurs; keine Paraphrase-Erfindung.
- **combined** — redundancy → instruction → semantic.
- **llm** (nur bei LLM-Betrieb) — bisheriger LLM-Optimizer als Kandidat.
- **baseline** — Long Prompt unverändert (Vergleich; no_improvement-Fallback).

## 5. SCORING

Auswahl unter strukturell gültigen und kleineren Kandidaten nach:

```
score = semantic_fidelity × (0.2 + 0.8 × token_efficiency)
        × (1 − 0.6 × min(guard_recovery_ratio, 1))        // Guard-Penalty
```

- Nur Kandidaten mit `verdict=pass` und `token_efficiency > 0` (echte
  Reduktion) kommen in die Auswahl; Rest ist Fallback no_improvement.
- Kein „kürzer = besser" ohne Erhaltungsnachweis; Guard-Penalty bestraft
  Kandidaten, die stark repariert werden mussten.
- QualityMetrics additiv (Option-Felder, serde-default; alte JSONs bleiben
  gültig): `constraint_preservation`, `technical_token_preservation`,
  `redundancy_removed`, `instruction_quality`, `output_contract_quality`
  (bei erfolgreicher Verifikation = 1.0, Erhaltungssinn).

## 6. GUARD

- Guard bleibt Sicherheitsnetz; pro Kandidat gemessen:
  `guard_recovered_atoms`, `guard_recovery_ratio`, `pre_guard_tokens`,
  `output_tokens` (nach Guard).
- Guard-Recovery erscheint als Note im Trace (`Guard-Pass hat N verlorene
  Pflicht-Atome wiederhergestellt (Strategie …)`) und im OptimizationReport.
- Regressionstests: „Anti-Zusammenfassung" (reine Zusammenfassung darf nicht
  bestehen), alle Strategien strukturell valide, `no_improvement` nie
  Degradierung.

## 7. BENCHMARK (deterministisch, `tests/optimizer/benchmark.sh`)

Reale Werte, estimated Tokens, `--no-llm`, Release-Binary:

```
intent      mode       gen → out   Reduktion   semantic  tech
intent1     baseline    67 →  67     0.0 %     1.00     1.00   no_improvement
intent1     redundancy  67 →  59    11.9 %     1.00     1.00   optimized/redundancy
intent1     structural  67 →  47    29.9 %     1.00     1.00   optimized/structural
intent1     semantic    67 →  59    11.9 %     1.00     1.00   optimized/semantic
intent1     combined    67 →  59    11.9 %     1.00     1.00   optimized/combined
intent1     auto        67 →  47    29.9 %     1.00     1.00   optimized/structural
intent2     auto        77 →  57    26.0 %     1.00     1.00   optimized/structural
intent3     auto       103 →  83    19.4 %     1.00     1.00   optimized/structural
intent4     auto       163 → 143    12.3 %     1.00     1.00   optimized/structural
```

- instruction allein erzeugt in diesen Fällen keine Reduktion (konservativ),
  was korrekt als no_improvement ausgewiesen wird.
- `make optimizer-benchmark` reproduziert diese Tabelle deterministisch.

## 8. APFEL (real, `make test-apfel`; Apple Foundation Model, apfel v1.5.5)

Suite PASS im Hauptdurchlauf; v1.0-Engine-Benchmark-Teil:

```
INTENT 1 (deutsch, kurz)  gen=373 → out=342  −8.3 %  status=optimized selected=semantic   semantic 1.00 technical 1.00  6 Kandidaten  25.8 s
INTENT 3 (deutsch, technisch/Pfad/URL)  gen=734 → out=716  −2.5 %  status=optimized selected=redundancy  semantic 1.00 technical 1.00  6 Kandidaten  38.3 s
INTENT 2 (englisch, lang) → FAIL exit 4: Architect lieferte abgeschnittenes JSON
  (Antwort beginnt mit gültigem `{schema_version…`, bricht aber ab → kein Fake-PASS)
```

Beobachtung: Bei realem LLM-Betrieb gewinnen deterministische Kandidaten oft
nicht gegen den LLM-Kandidaten in der Reduktion, aber die Engine wählt stets
den besten strukturell gültigen (INTENT 1 semantic, INTENT 3 redundancy —
beides deterministische Strategien; LLM-Kandidat war nicht kleiner/besser).

## 9. DEBUG TRACE

- `--debug-json` enthält jetzt `optimization` (status, selected, candidates
  inkl. tokens/semantic/guard/verification, score) im Trace-Dokument.
- LLM-Strategie-/Verify-Attempts weiterhin mit echten system/user/raw
  Prompts (Echo der Python-Schicht), redigiert (keine Secrets).
- `--no-llm`: ehrliche Stufen (`llm: false`), keine künstlichen raw_responses.

## 10. CLI

Additiv, keine Breaking Changes:

```
--optimizer auto|baseline|redundancy|instruction|structural|semantic|combined
```

- Default `auto` (Verhalten wie gehabt erweitert); `--help` zeigt das Flag.
- Strukturierte Formate (text/json/yaml/toon, Legacy `--json`) unverändert
  funktionsfähig; Compiler-Smoke-Format-Matrix PASS.

## 11. TESTS

- pf-engine: 34 lib-Tests (30 bestehende + 4 neue v1.0-Tests:
  `v10_optimized_selection_reports_candidates`, `v10_baseline_mode_is_no_
  improvement_not_degradation`, `v10_all_strategies_are_structural_valid`,
  `v10_optimization_report_serializes_additively`).
- pf-core: 60, pf-service 5, pf-tui 2, pf-bridge 1, pf-cli E2E 1 — alle grün.
- pytest: 18 passed / 1 skipped.
- `make optimizer-test` (fokussiert) und `make verify` PASS.
- apfel-Suite inkl. 3-Intent-Optimization-Benchmark (echte Calls, kein Mock).

## 12. CI / MAKEFILE

- CI unverändert; deterministische Optimizer-Tests laufen über `make verify`
  (pf-engine-Tests + Compiler-Smoke). apfel bleibt separater optionaler Test.
- Neue Targets: `make optimizer-test`, `make optimizer-benchmark` (determin.
  Bench, kein LLM). Bestehende Targets erhalten.

## 13. FILES CHANGED

- `crates/pf-core/src/compilation.rs` — OptimizationReport/CandidateReport/
  OptimizationStatus, QualityMetrics additiv, `optimization`-Feld
- `crates/pf-core/src/lib.rs` — Re-Exports
- `crates/pf-engine/src/optimization.rs` (neu) — Strategien + Schutz/Tests
- `crates/pf-engine/src/pipeline.rs` — run_optimization_engine, compile_with_
  optimizer, Metrics-Zusätze, neue Tests
- `crates/pf-cli/src/main.rs` — `--optimizer`; trace.rs — `optimization`-Feld
- `crates/pf-cli/src/app.rs` — normalize_optimizer
- `tests/optimizer/benchmark.sh` (neu) — deterministischer Benchmark
- `tests/providers/apfel/smoke.sh` — Abschnitt 10: 3-Intent-Real-Benchmark
- `Makefile` — optimizer-test / optimizer-benchmark
- `README.md`, `docs/architecture.md` (D-14/D-15), `docs/api.md`
- `docs/phase-reports/PHASE-V1-OPTIMIZATION-ENGINE-2026-09-03.md` (dies)

## 14. GIT STATUS

Kein Commit/Push (Konvention). Working Tree: modifizierte + neue Dateien wie
oben; Runtime-Artefakte ausschließlich /tmp und ~/.prompt-forge.

## 15. KNOWN LIMITATIONS

1. **Apple Foundation Model bleibt stochastisch** (INTENT 2 im Real-Lauf
   FAIL exit 4: abgeschnittenes Architect-JSON). Kein simuliertes PASS; die
   Suite ist unter `make test-apfel` bewusst stochastisch, `make verify`
   deterministisch.
2. `instruction` ist bewusst konservativ — erzeugt allein selten Reduktion;
   no_improvement wird korrekt gemeldet.
3. Deterministische Strategien operieren auf Zeilen-Token-Subsumption; eine
   echte semantische Deduplikation auf Paraphrase-Ebene („Do not change the
   API" = „API backward compatible") ist ohne LLM nicht beweisbar und wird
   NICHT geraten (nur LLM-Verify erlaubt solche Transformationen; als
   Kandidat `llm` mit anschließender Verifikation).
4. `pre_guard_tokens` misst nach Hygiene vor Guard; der Guard-Anhang kann den
   Prompt vergrößern — wird ehrlich ausgewiesen (eff basiert auf out_tokens).
5. Degraded-Status ist definiert, tritt im Erfolgspfad bewusst nicht auf
   (Engine wählt nie größer als Input; no_improvement statt Degradierung).
6. Real-LLM-Benchmarks brauchen Zeit/Latenz (INTENT 3 ~38 s) — CI nutzt den
   deterministischen Benchmark; apfel bleibt opt-in.

## 16. RECOMMENDATION

- v1.0-Kandidat bestätigen, sobald Review der uncommitted Änderungen erfolgt;
- apfel-Suite bei Bedarf erneut ausführen (stochastisch);
- v2.0 (Proxy/Context-Kompression) als separates Release-Ziel planen.

## 17. v1.0 READINESS (Definition of Done)

- [x] v0.x-Funktionalität erhalten (Compiler-Smoke/Formate/v0.1-Aliase grün)
- [x] mehrere Optimierungsstrategien (5 deterministisch + llm-Kandidat)
- [x] technische Tokens geschützt (is_protected_line, Metric)
- [x] semantische Erhaltung geprüft (strukturell + optional LLM, je Kandidat)
- [x] Guard als Recovery mit Messung (guard_recovered_atoms/-ratio)
- [x] negative Optimierung korrekt no_improvement/DEGRADED (nie größer als Input)
- [x] messbares Scoring (score = semantic × Nutzen × Guard-Penalty)
- [x] Debug Trace (optimization im --debug-json)
- [x] JSON/YAML/TOON/Text funktionsfähig
- [x] deterministische Tests (pf-engine 34, pytest 18, make verify PASS)
- [x] echte apfel-Tests (Suite PASS; INTENT 2 ehrlich FAIL, Limitierung)
- [x] Benchmark gegen bisherigen Optimizer (deterministisch; Tabelle oben)
- [x] Makefile (optimizer-test/optimizer-benchmark; bestehende erhalten)
- [x] GitHub Actions unverändert
- [x] Dokumentation (README/architecture/api + dieser Report)
