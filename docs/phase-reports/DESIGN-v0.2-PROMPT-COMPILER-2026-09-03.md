# PromptForge v0.2 — Design: The Prompt Compiler

Datum: 2026-09-03 · Status: Design (keine Implementierung) · Baseline: 4c074fb + ebcc0bb (unverändert)

---

## 1. Executive Summary

PromptForge v0.1 ist ein funktionierender Prompt-Compiler-Optimizer mit klarer
Rust/Python/PyO3-Architektur, Prompt IR, Expansion, Optimierung, Verifikation,
Token-Accounting und CLI/TUI/Service. v0.2 macht den „Compiler“-Charakter
explizit: Der Benutzer beschreibt **die Aufgabe** („auditiere das projekt“),
PromptForge kompiliert daraus **den Prompt**.

Dieses Dokument empfiehlt eine **additive, migrationsfreundliche** v0.2-
Architektur:

- Zwei neue Pipeline-Enden: **Intent Analysis** (vor dem Architect) und
  **Output Serialization** (nach der Verifikation).
- Ein einheitliches Ergebnisobjekt **CompilationResult** (additiv zum
  bestehenden JSON-Vertrag — v0.1-CLI/API/Tests bleiben kompatibel).
- **Format-Schicht** (text/json/yaml/toon) als eigene Serializer-Abstraktion,
  sauber getrennt in „strukturierte IR“, „executable prompt“ und „Envelope“.
- Zwei LLM-Rollen konzeptionell: **Prompt Generator** (Pipeline-LLM) und
  **Target LLM** (Zielmodell, an das der Prompt später geht) — Konfiguration
  ohne Bruch der bestehenden `LLM_ENDPOINT/LLM_KEY/LLM_MODEL`.
- **Qualitätsmodell** mit mehreren Achsen; eine Optimierung, die den Prompt
  vergrößert (token_efficiency < 0), gilt nicht automatisch als Erfolg.
- **Kein Big-Bang**: Rust/Python/PyO3/any-llm bleiben; die bestehende
  Pipeline wird erweitert, nicht ersetzt. Empfehlung weicht bewusst an einigen
  Stellen von der Aufgaben-Skizze ab (siehe §22 „Architecture Decision“).

Wichtigste Eigenempfehlungen (nicht bloß Bestätigung der Vorgabe):

1. **`--format` serialisiert ein Envelope-Dokument (CompilationResult)**, nicht
   die nackte IR; `text` ist der Sonderfall „executable prompt“. Damit ist
   `--json` (v0.1) exakt `--format json` und bestehende Skripte bleiben gültig.
2. **Serialisierung gehört nach Rust (pf-core)** — sie ist deterministisch,
   testbar und unabhängig vom LLM-Layer. Kein Python-Umweg über PyO3 für
   Formate.
3. **Intent Analysis in v0.2 ist eine schlanke LLM-Aufgabe mit
   deterministischem Fallback und deterministischer Validierung** — keine neue
   LLM-Operation nötig, solange die IR zusätzliche Analysefelder bekommt.
   Profile/Templates sind **kein v0.2-Muss** (Extension Point reicht).
4. **TOON wird unterstützt, aber als Envelope/IR-Darstellung, nicht als
   Prompt-Prosa.** Der IR-Aufbau ist nur mäßig tabular-freundlich; realistisch
   sind 10–35 % Token-Ersparnis. TOON-Implementierung erst nach Prüfung der
   Crate-Reife (Ökosystem konsolidiert sich gerade; Fallback: kleiner
   deterministischer Encoder hinter dem Serializer-Trait).
5. **Kein /v2-API-Zwang**: Die Service-API wird additiv erweitert; ein
   Breaking-Wechsel ist in v0.2 unnötig.

---

## 2. Current v0.1 Architecture

Stand (Commits 4c074fb, ebcc0bb; Working Tree sauber):

```text
CLI/TUI/Service ──► Engine (Rust, pf-engine)
                        │ architect (LLM via Pf-Bridge)  ODER deterministisch
                        ▼
                      Prompt IR (pf-core, JSON, schema_version=1)
                        │ expand (deterministisch, Rust)
                        ▼
                      Long Prompt
                        │ optimize (LLM-Pass + deterministische Passes + Guard)
                        ▼
                      Optimized Prompt
                        │ verify (strukturell deterministisch + LLM-Semantik;
                        │         Re-Optimize bis max_attempts)
                        ▼
                      CompileOutcome { ir, long_prompt, optimized_prompt,
                                       token_report, verification, … }
```

Bausteine:

- pf-core: Prompt IR, Config, PfError, Tokenizer (Heuristik, estimate),
  Persistenz/History, Redaction, Clipboard, HomeLayout (~/.prompt-forge).
- pf-engine: Engine, Expansion, Optimizer-Passes, Verifikation, MockBridge.
- pf-bridge: eingebettetes Python; JSON-Vertrag `promptforge.bridge.
  handle_request` mit Operationen `architect|optimize|verify|chat`.
- Python (promptforge): any-llm-Kapselung (create_openai_compatible), Mock,
  System-Prompts, IR-Normalisierung (robust gegen kleine Modelle).
- pf-cli (Binary `prompt-forge`): init/compile/serve/tui; compile kennt
  bereits `--json`, `-o`, `--copy`, stdin/Datei, `--no-llm`, Provider-Overrides.
- pf-service (axum): /v1/compile|optimize|verify|execute|health.
- pf-tui (ratatui): Intent-Input + Pipeline/Ergebnis-Ansicht.
- Tests: Rust unit/integration, Python pytest, E2E Python-Bridge,
  tests/providers/apfel/smoke.sh (echter LLM).

Eigenschaften, die v0.2 bewusst **behält**: transaktionale Boundary, Mock-
Testbarkeit, Guard-Pass, Retry-Loop mit Limit, Redaction, Rolling-Logs,
`~/.prompt-forge`-Layout, Heuristik-Tokenizer mit „estimate“-Kennzeichnung.

---

## 3. v0.2 Goals

1. „auditiere das projekt“ → fertiger, hochwertiger Prompt (kein Meta-Prompt-
   Schreiben durch den Benutzer).
2. Klare konzeptionelle Trennung **Prompt Generator** vs. **Target LLM**.
3. Output-Formate **text/json/yaml/toon** über eine einheitliche
   Serialisierungs-Schicht, mit sauberer Trennung IR/executable/Envelope.
4. Robustheit gegenüber realen (kleinen) LLMs: deterministische Validierung +
   Normalisierung + Qualitätsmessung, statt blindem LLM-Vertrauen.
5. Kein Bruch: v0.1-CLI, -API, -IR, -Konfiguration, -Tests bleiben
   weitestgehend kompatibel.
6. Basis für spätere Profile/Skills/Templates, ohne jetzt feste Listen zu
   erzwingen.

---

## 4. Proposed Architecture

```text
┌────────────────────────────────────────────────────────────────┐
│ Input Layer      CLI argument · stdin ("-") · Datei · API · TUI │
└──────────────┬─────────────────────────────────────────────────┘
               ▼
┌─────────────────────────────┐   deterministisch: Parsing, Validierung
│ Intent Analysis (neu, dünn) │   LLM: Intention erkennen/ergänzen
│  intent_text → Intent       │   Fallback: deterministische Basis-IR
└──────────────┬──────────────┘
               ▼
┌─────────────────────────────┐   zentrale semantische Wahrheit
│ Prompt IR (erweitert,       │   (JSON-Datenmodell, versioniert)
│  additiv, serde-defaults)   │
└──────────────┬──────────────┘
               ▼
┌─────────────────────────────┐   deterministisch (Rust)
│ Compiler Engine             │   architect/expand/optimize/verify wie v0.1,
│  (pf-engine, erweitert)     │   + Qualitäts-Metriken (neu)
└──────────────┬──────────────┘
               ▼
┌─────────────────────────────┐   deterministisch (Rust, pf-core)
│ Serializer Layer (neu)      │   text (executable prompt) · json · yaml ·
│  CompilationResult → output │   toon  (Envelope bzw. IR-Struktur)
└──────────────┬──────────────┘
               ▼
┌─────────────────────────────┐
│ Output Layer  stdout · Datei │ Clipboard · History · Service-Response
└─────────────────────────────┘
```

Datenfluss (Kern):

```text
intent_text ─► Intent ─► PromptIr ─► expanded ─► optimized ─► verified
   ─► CompilationResult { intent, prompt_ir, expanded_prompt, optimized_prompt,
                          verification, metrics, output_format, final_output }
   ─► Serializer(format) ─► text | json | yaml | toon
```

Änderungen je Schicht:

| Schicht | Status in v0.2 |
|---|---|
| pf-core | + Intent-Typen, + CompilationResult, + Serializer-Trait (text/json/yaml/toon), + Qualitäts-Metriken-Typen; IR additiv erweitert |
| pf-engine | + Intent-Analyse-Stufe (LLM via bestehender Bridge-Operation `architect` oder neue leichte Operation `intent`), Pipeline-Endstufe „serialize“, Metriken-Berechnung |
| pf-bridge | unverändert (JSON-Vertrag bleibt; optional neue Operation `intent`) |
| python | unverändert bis leicht (System-Prompt des Architect um Intent-Analyse-Felder ergänzen; optional eigener Intent-Prompt) |
| pf-cli | `--format`/`--out`-Semantik, stdin `-`; bestehende Flags bleiben |
| pf-service | /v1/compile additiv (`input`, `format`, `profile` optional), Response-Envelope erweitert |
| pf-tui | Format-Wahl + Ergebnis-Ansicht (später, Phase 6) |
| Storage | vorhandenes Layout; History um format/metrics ergänzt |

---

## 5. Prompt IR

Bestand (v0.1, schema_version=1): task, objective[], context[], inputs[],
constraints[], assumptions[], role, procedure[], reasoning_strategy,
examples[], output_contract{}, verification_requirements[], target_model,
metadata{}.

Bewertung: Die IR ist für v0.2 **weitgehend ausreichend**. „instructions“
(vorgegebener Begriff) bilden wir auf `procedure[]` + `task` ab; eine
zusätzliche Unterscheidung ist nicht zwingend.

Empfohlene additive Erweiterung (alle Felder optional + serde-Defaults, damit
v0.1-IR-Dokumente weiter parsen; `schema_version` bleibt `1` auf der Leitung,
intern Version 2 mit Default „wenn 1, dann alte Felder“):

```rust
pub struct IntentAnalysis {            // neu, optional in PromptIr
    pub task_type: Option<String>,     // freie Kategorie, KEINE feste Liste
    pub profile_hint: Option<String>,  // später: Templates/Skills
    pub language: Option<String>,      // Sprache des Intents
    pub confidence: Option<f64>,       // LLM-Konfidenz (0..1)
    pub notes: Vec<String>,            // Verständnis-/Ambiguitätsnotizen
}

pub struct PromptIr {
    // … bestehende Felder unverändert …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<IntentAnalysis>,   // neu
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,                  // neu (freie Tags statt Taxonomie)
}
```

Begründung: keine neue Pflichtstruktur; kleine Modelle liefern unvollständige
Felder, daher bleibt die Python-Normalisierung (v0.1) zuständig und füllt
Defaults. Die IR ist bewusst **formatunabhängig** (JSON-Datenmodell); text/
json/yaml/toon sind Serialisierungen, kein IR-Ersatz.

---

## 6. Intent Layer

Ziel: `Intent { raw_text, normalized_text, analysis, profile_hint?, meta }`.

Empfehlung: Intent ist **kein eigenes IR-Duplikat**, sondern eine schlanke
Eingangs-Struktur, die in die PromptIr mündet:

- **Deterministisch**: Trimmen, Sprache-Hinweis (einfache Heuristik), Länge,
  Keyword-Hinweise („audit“, „security“, „test“ → task_type-Vorschlag, nur als
  Hinweis, Konfidenz niedrig), leere Eingabe → Fehler.
- **LLM (Prompt Generator)**: Der bestehende Architect-System-Prompt wird um
  die Analyse-Felder erweitert (task_type, language, notes, confidence).
  Alternativ eigene Operation `intent` — empfohlen: **zunächst im
  architect-Aufruf**, weil die Bridge-Operationen stabil bleiben und ein
  LLM-Roundtrip gespart wird; `intent` als eigene Operation erst, wenn Profile
  (v0.3) das erfordern.
- **Fallback ohne LLM**: deterministische Basis-IR (v0.1 `from_intent_basic`)
  plus task_type aus Keyword-Matcher.

Beispiel „auditiere das projekt“ → LLM-Architect erzeugt IR mit
`analysis.task_type≈"audit"`, Rolle Senior-Software-Architect/Reviewer,
procedure-Schritten, constraints (keine erfundenen Befunde, Evidenz), etc.
Die konkrete Ausprägung bleibt Modell-/Prompt-Sache — die IR-Struktur ist die
Garantie.

---

## 7. Compiler Pipeline

Gegenüber v0.1 ändert sich der Ablauf nur an zwei Stellen:

```text
[NEU] intent_analysis: intent_text → Intent → PromptIr
      (LLM architect erweitert ODER deterministisch)

v0.1-Pipeline unverändert:
      architect → expand → optimize(→verify-retry) → verification

[NEU] serialize: CompilationResult → Serializer(format)
[NEU] quality:   QualityMetrics (in CompilationResult)
```

Wiederverwendung: Engine.compile() bleibt Einstieg; intern bekommt sie eine
`IntentAnalyzer`- und eine `Serializer`-Abhängigkeit (Trait-Objekte), damit
CLI/Service/TUI identisch bleiben und Tests Mocks injizieren können.

Wiederverwendbare Komponenten: expand, optimizer passes, guard, verify,
tokenizer, persistenz. Unverändert: bridge/py, Redaction, Logging, Layout.

---

## 8. LLM Roles

Konzeptionell zwei Rollen:

- **Prompt Generator**: LLM innerhalb der Pipeline (architect/optimize/
  verify). Das ist heute `LLM_ENDPOINT/LLM_KEY/LLM_MODEL` (any-llm).
- **Target LLM**: Modell, an das der fertige Prompt später gesendet wird.
  In v0.2 ist das **Metadatum + Konfigurationsziel** (Envelope-Feld
  `target`/`target_model`); eine spätere Ausführung (`prompt-forge run` oder
  `POST /v1/execute` mit Target-Profil) ist dadurch vorbereitet.

Empfohlene Konfiguration (siehe auch §11): getrennte Profile unter
`~/.prompt-forge/config/` und Env-Präfixe:

```toml
[generator]            # = Pipeline-LLM (heute LLM_*)
endpoint = "…"         # optional
key = "…"              # optional
model = "…"
timeout_s = 120

[target]               # Zielmodell (v0.2: Metadaten)
endpoint = "…"         # optional
model = "…"            # optional
```

Env: `PROMPTFORGE_GENERATOR_ENDPOINT/KEY/MODEL`,
`PROMPTFORGE_TARGET_ENDPOINT/KEY/MODEL`.
**Kompatibilitätsregel:** Solange `[generator]`/`PROMPTFORGE_GENERATOR_*`
nicht gesetzt sind, gelten `LLM_ENDPOINT/LLM_KEY/LLM_MODEL` als Generator —
v0.1-Verhalten unverändert. `target_model` in der IR/Envelope kommt aus
`[target]` bzw. `PROMPTFORGE_TARGET_MODEL`, sonst bleibt es wie v0.1.

---

## 9. Output Formats — drei Konzepte sauber trennen

Empfohlene Trennung:

- **(A) Structured representation**: die Prompt IR selbst (internes
  JSON-Datenmodell; serialisiert z. B. als json/yaml/toon). Persistenzformat
  bleibt JSON (v0.1).
- **(B) Executable prompt**: der finale Prompt als Prosa-Text für ein
  Ziel-LLM — das ist `text`.
- **(C) Envelope**: Dokument mit Ergebnissen/Metadaten:
  `{ input, intent, prompt_ir, expanded_prompt, optimized_prompt,
     verification, metrics, output_format, final_output }` = CompilationResult.

Konsequenz für die CLI: `--format` wählt die Darstellung des **Envelope**
(CompilationResult). `text` gibt nur `final_output` (executable prompt) aus —
das v0.1-Standardverhalten für Skripte bleibt damit erhalten. Wer die reine
IR-Struktur will, nutzt json/yaml/toon des Envelope bzw. (später)
`--format json --field prompt_ir` oder ein eigenes `prompt-forge ir`-Kommando.

Empfehlung statt „--field“-Komplexität in v0.2: json/yaml/toon liefern den
kompletten Envelope; die IR liegt darin als Feld `prompt_ir`.

---

## 10. text / json / yaml / toon

### text
Executable prompt (optimierter Prompt; v0.1-Standard). Deterministisch; kein
Serializer im engen Sinn, sondern Selektion `optimized_prompt`.

### json
Envelope als JSON. In v0.1 existiert `--json` bereits mit Feldern
request_id/llm_used/stages/long_prompt/optimized_prompt/token_report/
verification (+ ir, saved). **Diese Schlüssel bleiben**; neue Felder
(intent, metrics, output_format, final_output) werden ergänzt. So bleiben
`tests/providers/apfel/smoke.sh` und bestehende Konsumenten gültig.

### yaml
Envelope als YAML (menschenlesbar für Config-ähnliche Nutzung). Rust:
`serde_yaml` ist wartungsarm; falls zum Implementierungszeitpunkt besser
gepflegt: `serde_yml`. Entscheidung bei Implementierung mit Begründung
(minimale Dependency, nur Serialize).

### toon
Recherche-Ergebnis (extern geprüft, nicht erfunden):

- **TOON = Token-Oriented Object Notation**, spezifikationsbasiert
  (spec v4.1, Repo toon-format/toon bzw. toon-format/spec), MIT.
- Kodiert das **JSON-Datenmodell deterministisch & verlustfrei**; Ziel:
  Token-Ersparnis + Struktur-Guardrails (`[N]`-Längen, `{felder}`-Header)
  für LLM-Konsum. Benchmark (Projektangaben): ~29 acc%/1K Tokens vs.
  JSON-compact ~24 — bei nicht-tief-verschachtelten Daten.
- Formen: inline (`alerts[2]: frost,wind`), tabular
  (`forecast[3]{day,temp{min,max},…}:` + Zeilen), keyed tabular
  (`envs[2:]{…}: key: …`), list (`- item`). Nicht-uniform/tief: JSON oft
  besser. Dateiendung `.toon`, Media-Type `text/toon` (provisional).
- Ökosystem **jung/consolidierend**: Rust crates.io `toon` (offiziell,
  „toon-format“ beansprucht), `toon-rs` (serde), Python `toon-format`-Port
  (beta). Versionsangaben schwanken (README v4.1 vs. Rust „v3.0“).

Design-Empfehlung PromptForge:

1. TOON ist **Serialisierung der strukturierten Darstellung** (Envelope/IR),
   nicht die Prompt-Prosa. Als LLM-Eingabe ist TOON dort sinnvoll, wo
   strukturierte Spezifikation (Output-Contract, Constraints-Tabellen) an ein
   Ziel-LLM gehen soll — das wird in v0.2 **nicht** erzwungen, bleibt aber
   durch das IR-JSON-Modell möglich (TOON ist verlustfreie JSON-Kodierung).
2. Vorteile ggü. JSON: Token-Ersparnis bei uniformen Listen (constraints[],
   inputs[], procedure[]-artige), Guardrails durch deklarierte Längen;
   Nachteile: Ökosystem-Reife, tiefe/nicht-uniforme Strukturen (unser IR hat
   einige optionale Felder → Ersparnis realistisch 10–35 %, nicht 40 %+).
3. Deterministisch: Serializer folgt der Spezifikation (feste Feldreihenfolge
   = JSON-Objekt-Reihenfolge der serde-Struktur; keine Sortierung einführen);
   Roundtrip-Tests json→toon→json (identisch).
4. Implementierung: hinter `Serializer`-Trait; bevorzugt offizielle Rust-Crate
   (`toon`/`toon-format`) — **Reife zum Implementierungszeitpunkt prüfen**
   (Conformance-Suite). Fallback: kleiner deterministischer Encoder in
   pf-core für die im IR vorkommenden Formen, gekennzeichnet als
   „subset + conformance pending“. Kein Vendor-Lock: Trait erlaubt Austausch.

---

## 11. Target Model Configuration

Empfehlung (s. §8): zwei Profile in `~/.prompt-forge/config/config.toml`
(`[generator]`, `[target]`) + Env `PROMPTFORGE_GENERATOR_*`/
`PROMPTFORGE_TARGET_*`, mit expliziter Kompatibilitätsregel, dass
`LLM_ENDPOINT/LLM_KEY/LLM_MODEL` den Generator definieren, solange kein
Generator-Profil gesetzt ist. Kein neues Secrets-Handling nötig (Redaction
übernimmt beide Profile; Log-Scrub-Liste um `PROMPTFORGE_*_KEY` erweitern).

Das ist bewusst **kein** Provider-System im Rust-Core — any-llm bleibt die
einzige LLM-Schnittstelle; „target“ ist zunächst Metadatum.

---

## 12. CLI Design

Beibehalten (kein Bruch): `prompt-forge compile "…"` (Default text → Prompt
auf stdout, Statistik stderr), `-o`, `--copy`, `--no-llm`, Provider-Overrides,
`--home`, `init/serve/tui`.

Neu/erweitert:

```bash
prompt-forge compile "auditiere das projekt"                 # text (default)
prompt-forge compile "…" --format json                      # Envelope JSON
prompt-forge compile "…" --format yaml                      # Envelope YAML
prompt-forge compile "…" --format toon                      # Envelope TOON
prompt-forge compile "…" --format text -o prompt.txt
prompt-forge compile -                                       # stdin
prompt-forge compile prompt.txt                              # Datei (v0.1)
```

- `--json` bleibt als Alias für `--format json` (Kompatibilität).
- `--format` ist der richtige Name: Er beschreibt die Darstellung des
  Ergebnisses; Alternativen (`--output-format`) wären redundant.
- Interaktives Menü (v0.1) bleibt nur für text ohne `--format`.
- Exit-Codes unverändert (0/2/3/4/5/6/7).

---

## 13. Service API

Bestehend: /v1/compile|optimize|verify|execute|health. Empfehlung: **additiv
erweitern, keine neue Version** (v0.1-Clients brechen nicht):

```json
POST /v1/compile
{ "input": "auditiere das projekt", "format": "text" }
// „intent“ bleibt als Alias für „input“ gültig (v0.1)
→ 200 {
  "request_id": "…",               // v0.1-Felder bleiben
  "llm_used": true,
  "stages": [ … ],
  "long_prompt": "…",
  "optimized_prompt": "…",
  "token_report": { … },
  "verification": { … },
  "ir": { … },
  // neu:
  "intent": { … },
  "metrics": { … },
  "output_format": "text",
  "final_output": "…"              // = optimized_prompt bei text
}
```

Bei `format: json|yaml|toon` liefert der Endpoint den serialisierten Envelope
(Content-Type entsprechend bzw. `application/json` für json). Fehler-Envelope
unverändert. `/v1/optimize|verify` bleiben; `/v1/execute` nutzt in v0.2
weiterhin den Generator (Semantik „Ziel-LLM ausführen“ erst mit Target-Profil,
v0.3).

---

## 14. TUI UX

Empfehlung (keine Implementierung in v0.2-Pflicht):

- Startansicht: ein Eingabefeld („What do you want the LLM to do?“), darunter
  Format-Auswahl `text|json|yaml|toon` (Tab oder Zahl), `[Compile]`.
- Ergebnisansicht: Registerkarten „Generated / Optimized / Verification /
  Metrics“; bei `json/yaml/toon` Vorschau des serialisierten Envelope.
- Recompile, Copy, Save wie v0.1; zusätzlich Format-Wechsel nach Lauf ohne
  Neu-Kompilierung (Serializer ist deterministisch → sofort).
- Architektur: TUI bleibt dünner Client der Engine (v0.1-Prinzip).

---

## 15. Storage

Kein neues Verzeichnis-Layout. Bestehendes `~/.prompt-forge/` wird genutzt:

```text
prompt/templates/    (später: Templates/Skills — heute leer)
prompt/generated/    long-…-<ts>.md, ir-…-<ts>.json
prompt/optimized/    optimized-…-<ts>.md      (+ bei Bedarf final-… im Archive)
prompt/archive/      unverändert
history/history.jsonl  Einträge um format/metrics/output_path erweitert
state/               unverändert
cache/, logs/, config/  unverändert
```

Empfehlung: „compiled prompts“ als normale Artefakte in
`prompt/optimized/` (formatabhängige Endung: `.md` für text; bei json/yaml/
toon zusätzlich Datei mit Endung) ablegen und im History-Eintrag referenzieren.
Persistenz bleibt atomar/idempotent (v0.1).

---

## 16. Templates / Profiles

Analyse: Profile (audit/security/research/…) sind nützlich, aber **kein
v0.2-Muss**: Die LLM-Architect-Pipeline kann „auditiere das Projekt“ bereits
ohne feste Liste korrekt expandieren. Feste Listen widersprechen dem Prinzip
„keine Taxonomie erzwingen“.

Empfehlung:

- **v0.2**: Extension Point, keine Profile. `IntentAnalysis.profile_hint`/
  `task_type` als freie Strings; der (optionale) deterministische
  Keyword-Matcher liefert nur Hinweise. Templates-Verzeichnis existiert
  bereits (`prompt/templates/`), wird aber nicht aktiv genutzt.
- **v0.3 (optional)**: Profil = (a) Prompt-Fragment/Seed für den Architect,
  (b) Output-Contract-Defaults, (c) Verify-Schwerpunkte; Registrierung als
  Dateien unter `~/.prompt-forge/prompt/templates/<profil>.{toml,md}` +
  `--profile`/Auto-Detect. Der Core braucht dafür keinen Umbau, nur einen
  `ProfileProvider`-Trait.

---

## 17. Verification

v0.1-Mechanik bleibt: strukturelle Atom-Checks (deterministisch) + LLM-
Semantik-Report, Re-Optimize mit Feedback bis `max_attempts`, Guard-Pass
(Objective/Constraints/Contract/Instructions/Requirements werden re-insertiert).

v0.2-Erweiterungen (additiv):

- Verifikation liefert weiterhin `VerificationReport`; neu ergänzt:
  `QualityMetrics` (s. §18), berechnet aus Report + Token-Report.
- „Verification PASS“ bleibt wie v0.1 definiert (Schwellen). Neu: wenn
  `token_efficiency` stark negativ ist, wird der Lauf als
  `PASS_WITH_CAVEAT`/`DEGRADED` gekennzeichnet (kein stiller Erfolg).
- Keine Endlosschleifen: Grenzen unverändert.

---

## 18. Optimization Quality

Konzeptionelles Qualitätsmodell (noch keine Implementierung):

```rust
pub struct QualityMetrics {
    pub semantic_fidelity: f64,        // = semantic_preservation (0..1)
    pub structural_validity: bool,     // strukturelle Atom-Checks ok
    pub token_efficiency: f64,         // relativ: 1 - optimized/generated
                                       //   > 0 kleiner; =0 gleich; <0 größer
    pub instruction_quality: Option<f64>,  // LLM-Einschätzung (optional)
    pub output_contract_quality: Option<f64>,
    pub verdict: QualityVerdict,       // Optimal | Acceptable | Degraded | Failed
}
```

Entscheidungslogik (Vorschlag):

```text
semantic_fidelity >= threshold UND structural_validity
        UND token_efficiency >= 0          → Optimal
semantic_fidelity >= threshold UND structural_validity
        UND token_efficiency > -0.15       → Acceptable
sonst bei erhaltener Semantik, aber größer → Degraded
verdict failed wenn semantic < threshold
```

Beispiel aus v0.1-Realität: `semantic_fidelity=1.00`, `token_efficiency=-0.42`
→ **Degraded** (Inhalt erhalten, aber vergrößert) — wird nicht als
erfolgreiche Optimierung verkauft; CLI meldet Metrik + Hinweis, Exit-Code
bleibt 0 (Ergebnis existiert), aber `make verify`-artige Gates können
`Degraded` als Warnung behandeln (konfigurierbar).

Wo berechnet: semantic/instruction/output_contract → hybrid (strukturelle
Atome deterministisch + LLM-Report); token_efficiency, structural_validity →
deterministisch.

---

## 19. Testing

### Unit (Rust)
- Intent: Normalisierung, Keyword-Hinweise, leer/ungültig.
- Prompt IR: additive Felder default-kompatibel (alte JSONs parsen),
  Roundtrip.
- Serializer: text/json/yaml/toon deterministisch; **Roundtrip
  json→toon→json == identisch**; yaml→json stabil; Feldreihenfolge stabil;
  Unicode/Quoting.
- Validierung: IR-Validierung mit/ohne Analyse-Felder.
- Metriken: token_efficiency/verdict-Matrix (inkl. -0.42-Fall).
- Engine: Intent-Stage mit MockBridge; Pipeline unverändert grün.

### Integration
- CLI: `--format`-Matrix auf deterministischem Pfad; `-`/Datei/`-o`;
  Alias `--json`.
- Service: /v1/compile mit `format` (Axum-Tests).
- Python bridge: Vertrag unverändert (Regression).
- LLM provider: Mock + any-llm-Wiring (unreachable probe wie bisher).

### Real LLM
- apfel (bestehender Smoke; env-gesteuert).
- Ollama/LM Studio/OpenAI-compatible: gleicher Smoke-Pfad über
  `APFEL_ENDPOINT`-Analogie bzw. LLM_ENDPOINT; **skip, wenn nicht verfügbar**;
  keine Credentials im Repo.

### Golden Tests
- Input „auditiere das projekt“ mit Mock-LLM → feste IR-Struktur-Asserts
  (task nicht leer, role gesetzt, procedure ≥3, constraints ≥1,
  output_contract.structure ≥1, analysis.task_type=="audit"-Hinweis bei
  Deterministic-Mode). Kein LLM-Text-Vergleich; deterministische
  Eigenschaften.

---

## 20. Backward Compatibility

| Bereich | Kompatibel? | Erläuterung |
|---|---|---|
| v0.1 CLI | ja | compile-Default text; `--json`=Alias; Flags bleiben |
| v0.1 API | ja | /v1/* Pfade + Request-/Response-Kernfelder unverändert; additiv |
| v0.1 Prompt IR | ja | alte JSONs parsen (serde-defaults); `schema_version` bleibt 1 on-wire |
| v0.1 Konfiguration | ja | LLM_ENDPOINT/KEY/MODEL = Generator-Default; neue Profile optional |
| v0.1 Tests | ja | Rust/Python/E2E/smoke.sh lesen bekannte Felder; neue Felder ignoriert |
| Storage | ja | gleiche Pfade; History additiv |
| Python-Bridge-Vertrag | ja | Operationen unverändert; Prompt-Inhalte erweitert |
| pf-tui | ja | bestehende Ansicht; Format-Wahl additiv |

Darf sich ändern: interne Engine-Signaturen (nicht public API), neue
Optionen, System-Prompt-Texte, Cargo-Dependencies (additiv), Exit-Verhalten
nur bei neuen Optionen.

---

## 21. Migration Plan

```text
v0.1 (stabil) ──► v0.2 (compiler)
```

Schritte (klein, einzeln testbar, additive Commits):

1. Commit „v0.2 foundation“: CompilationResult + QualityMetrics-Typen in
   pf-core (unbenutzt? nein: engine gibt sie zusätzlich aus), Intent-Typen,
   CLI/Service unverändert — nur Typen + Tests. → kein Verhaltensbruch.
2. Commit „serializer“: Serializer-Trait + text/json/yaml (+toon hinter
   Trait), `--format`, Alias `--json`, Envelope-Felder final_output etc.
   Golden-Tests.
3. Commit „intent analysis“: architect-System-Prompt um analysis-Felder
   erweitern (Python), IntentAnalyzer (LLM-optional) in Engine, Fallback.
4. Commit „metrics & verdict“: QualityMetrics-Berechnung, DEGRADED-Kennzeich-
   nung, CLI-Ausgabe.
5. Commit „service/tui“: /v1/compile format-Parameter, TUI-Format-Auswahl.
6. Commit „tests/docs“: Ollama/LM-Studio-Smoke-Env, Golden-Tests, README/
   docs.

Jeder Schritt lässt `make verify` (ohne apfel) grün.

---

## 22. Recommended v0.2 Architecture — Entscheidungen & Begründung

RECOMMENDED V0.2 ARCHITECTURE (Zusammenfassung mit Datenfluss s. §4):

```text
Input Layer (CLI/API/TUI) ──► Intent Layer (deterministisch + LLM via
    bestehender architect-Operation) ──► Prompt IR (additiv) ──► Compiler
    Engine (v0.1-Pipeline + Qualität) ──► CompilationResult ──► Serializer
    Layer (text/json/yaml/toon, Rust) ──► Output (stdout/Datei/Clipboard/
    History/API) · LLM Layer (any-llm; Generator + Target-Metadaten) ·
    Storage (unverändert) · CLI/TUI/Service = dünne Clienten der Engine.
```

Begründete Abweichungen von der Aufgaben-Skizze (bewusste Empfehlungen):

1. **Serializer in Rust statt „Format egal wo“**: deterministisch, testbar,
   kein PyO3-Roundtrip für Ausgabe; Text ist Selektion, json/yaml/toon sind
   Envelope-Serialisierer. (Skizze lässt offen; Rust ist die bessere Wahl.)
2. **Intent Analysis ohne neue LLM-Operation**: Erweiterung des bestehenden
   architect-Aufrufs spart Roundtrip und hält die Bridge stabil. Eigene
   Operation erst bei Profilen.
3. **Envelope-Format-Semantik**: `--format json` == v0.1 `--json` (Envelope),
   nicht nackte IR — Kompatibilität und Vollständigkeit; wer nur die IR will,
   bekommt sie als Feld.
4. **Profile erst v0.3**: Extension Point jetzt, Listen später — verhindert
   Taxonomie-Zwang und hält v0.2 klein.
5. **TOON als Envelope/IR-Darstellung, nicht als Prompt-Prosa**: entspricht
   dem Format (JSON-Datenmodell) und unserer Drei-Konzepte-Trennung.
6. **Kein /v2**: additive API-Erweiterung reicht; Versionierung erst bei
   echten Brüchen.

Matrix deterministisch vs. LLM (mit Begründung):

| Aufgabe | Deterministisch | LLM | Begründung |
|---|---|---|---|
| Parsing (Input/CLI/JSON) | ✓ | | Syntax, keine Semantik |
| IR validation | ✓ | | Struktur-/Pflichtfeld-Checks sind Regeln |
| Serialization | ✓ | | Reproduzierbarkeit, Tests, Roundtrip |
| Token accounting | ✓ | | Heuristik deterministisch (estimate markiert) |
| Intent interpretation | | ✓ | Bedeutung braucht LLM |
| Intent normalization/Detect-Hints | ✓ | | Keywords/Sprache-Heuristik als Hinweis |
| Prompt expansion (LLM-Architect) | | ✓ | Semantische Anreicherung |
| Expansion-Rendering (v0.1) | ✓ | | IR→Text-Vorlage |
| Semantic verification | | ✓ | Bedeutungserhalt bewerten |
| Schema validation | ✓ | | JSON-Schema/Struktur |
| Optimization (Rewrite) | | ✓ | Sprachliche Kompression |
| Optimizer-Hygiene/Guard | ✓ | | Whitespace/Dedupe/Reinsert |
| Quality scoring | ggf. | ggf. | Metriken deterministisch; semantic via LLM-Report |

---

## 23. Open Questions

1. TOON-Crate-Reife zum Implementierungszeitpunkt (offiziell `toon`/
   `toon-format` vs. Eigen-Encoder); Conformance-Suite verfügbar?
2. YAML-Dependency-Wahl (serde_yaml Pflegezustand vs. Alternative) — minimale
   Serialize-only-Nutzung.
3. Soll `text` in v0.2 zusätzlich eine „explain“-Variante (Prompt + kurze
   Strukturerklärung) bieten? (Vorschlag: nein, YAML/TOON decken das ab.)
4. Target-LLM: nur Metadaten (v0.2) oder schon `prompt-forge run`/execute
   gegen Target-Profil? (Empfehlung: Metadaten; run v0.3.)
5. Envelope-Schema-Versionierung: Feld `schema_version` im Envelope einführen?
   (Empfehlung: ja, default 2, additiv.)
6. `--format json --field …`-Filterung später? (Offen, v0.3.)
7. Auto-Profile-Detection: deterministisch genug? (v0.3-Entscheidung.)
8. Quality-Gates: Soll `Degraded` einen Exit-Code ungleich 0 bekommen oder
   nur Warnung? (Empfehlung: Warnung + konfigurierbares Gate in make verify.)

---

## 24. Recommended Implementation Phases

Phase 0 — Architecture (dieses Dokument; Review)
Phase 1 — Intent Input & CompilationResult-Typen (Rust; additive Typen +
         Tests; CLI/Service unverändert)
Phase 2 — Serializers text/json/yaml (+toon) + `--format`/`--json`-Alias +
         Envelope-Felder; Golden-/Roundtrip-Tests
Phase 3 — Intent Analysis (Python-Prompt + IntentAnalyzer + Fallback;
         IR-Felder analysis/tags)
Phase 4 — Quality Metrics & Verdict (inkl. DEGRADED, CLI-Ausgabe)
Phase 5 — Service (/v1/compile format/input) + TUI-Format-Auswahl
Phase 6 — Real-LLM-Tests (apfel bestehend; Ollama/LM Studio env-gesteuert,
         skip wenn fehlend)
Phase 7 — Docs (README/docs/architecture.md/docs/api.md,
         ggf. docs/formats.md) & Kompatibilitäts-Regression (make verify)

Jede Phase endet mit grünen Gates (`cargo test/clippy/fmt`, `uv run pytest`,
CLI-Smoke deterministisch) und optionalem apfel-Smoke.

---

*Ende Design-Bericht v0.2.*
