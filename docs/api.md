# PromptForge HTTP-API (v0.1)

`prompt-forge serve` startet den lokalen Service (Default
`http://127.0.0.1:8770`, konfigurierbar: `PF_SERVICE_HOST`, `PF_SERVICE_PORT`
bzw. `[service]` in config.toml).

Alle Endpunkte sprechen JSON. Fehler haben einen einheitlichen Envelope:

```json
{
  "kind": "verification|configuration|provider|…",
  "message": "menschenlesbar",
  "retryable": false
}
```

HTTP-Status: `400` ungültige Eingabe/Konfiguration, `422` Optimierungs-/
Verifikationsfehler, `502` LLM-/Provider-Fehler, `500` intern.

## GET /v1/health

Service-Status (Version, effektiver Provider, Modell).

## POST /v1/compile

Vollständige Pipeline: Intent → IR → Long Prompt → Optimierung → Verifikation.

Request (Default, v0.1-kompatibel):

```json
{ "intent": "Analysiere diese fünf Papers, vergleiche die Methoden …" }
```

Request (v0.2, additiv): `input` als Alias für `intent` sowie optionales
`format` (text|json|yaml|toon). Mit `format` liefert der Endpoint
`{format, output, input}` statt des Envelope:

```json
{ "input": "auditiere das projekt", "format": "yaml" }
```

Response 200 (Default, ohne `format`):

```json
{
  "request_id": "…",
  "llm_used": true,
  "stages": ["architect", "expand", "optimize", "verify"],
  "long_prompt": "# Prompt\n## Aufgabe …",
  "optimized_prompt": "…",
  "token_report": {
    "original": 14, "generated": 620, "optimized": 290,
    "estimate": true, "stages": []
  },
  "verification": {
    "semantic_preservation": 0.96,
    "constraints_preserved": true,
    "output_contract_preserved": true,
    "objective_preserved": true,
    "instructions_preserved": true,
    "verdict": "pass",
    "checks": [ … ], "attempts": 1, "details": [ … ]
  }
}
```

Bei nicht bestandener Verifikation nach `max_attempts`: HTTP 422 mit
`kind: verification`.

Hinweis: Der CLI-Debug-Trace (`prompt-forge compile … --debug` bzw.
`--debug-json -o debug.json`, Details README) ist eine CLI-Funktion und
betrifft die API-Antworten nicht; `/v1/compile` liefert weiterhin den
Envelope bzw. bei `format` das `{format, output, input}`-Dokument.

## POST /v1/optimize

Optimiert einen vorhandenen Long Prompt zu einer IR (mit optionalem
Re-Optimize-Feedback aus einer früheren Verifikation).

Request:

```json
{
  "ir": { "task": "…", "constraints": [ … ], "output_contract": { … } },
  "long_prompt": "…",
  "feedback": ["[constraint] Nur peer-reviewte Quellen verwenden"]
}
```

Response 200:

```json
{ "optimized_prompt": "…", "verification": { … }, "optimizer_passes": ["normalize_whitespace", …] }
```

## POST /v1/verify

Verifiziert einen optimierten Prompt gegen Original + IR (strukturell +
optional LLM-Semantik).

Request:

```json
{
  "ir": { … },
  "long_prompt": "…",
  "optimized_prompt": "…"
}
```

Response 200: `VerificationReport` (siehe oben), `verdict: "pass"|"fail"`.

## POST /v1/execute

Führt einen fertigen Prompt gegen das Ziel-LLM aus (verlangt konfiguriertes
LLM; Provider/Model aus Umgebung/Config).

Request:

```json
{ "prompt": "…" }
```

Response 200:

```json
{
  "content": "Antwort des LLM",
  "model": "…",
  "finish_reason": "stop",
  "usage": { "prompt_tokens": 100, "completion_tokens": 50 },
  "duration_ms": 1234
}
```

## Sicherheit

- Der Service bindet per Default nur an `127.0.0.1`.
- Logs enthalten keine API-Keys/Authorization-Werte (Redaction), keine
  vollständigen Prompts (nur opt-in `PF_PROMPT_LOG=1`).
- Für Remote-Zugriff: Reverse-Proxy mit Auth; Secrets gehören in die
  Umgebung, nicht in Requests.

## v1.0: Optimization-Report in strukturierten Formaten

Seit v1.0 enthält der CompilationResult-Envelope (json/yaml/toon) zusätzlich
den additiven Schlüssel `optimization` (nur wenn die Engine lief):

```json
"optimization": {
  "input_tokens": 250,
  "baseline_tokens": 250,
  "optimization_status": "optimized",
  "selected": "structural",
  "score": 0.98,
  "guard_recovered_atoms_total": 0,
  "candidates": [
    { "strategy": "redundancy", "input_tokens": 250, "pre_guard_tokens": 210,
      "output_tokens": 205, "token_efficiency": 0.18,
      "semantic_fidelity": 0.99, "structural_validity": true,
      "verification": "pass", "guard_recovered_atoms": 0,
      "guard_recovery_ratio": 0.0 }
  ]
}
```

`optimization_status`: `optimized` | `no_improvement` | `degraded`. Bei
`no_improvement` ist `optimized_prompt` der Long Prompt (keine künstliche
Verschlechterung). Der Report erscheint identisch im Debug-Trace-Dokument
(`--debug-json`).

CLI: `compile --optimizer auto|baseline|redundancy|instruction|structural|semantic|combined`
(Default `auto`). Kein Breaking Change — strukturierte Antworten ohne Engine-
Report sind weiterhin gültig (Feld fehlt dann einfach).

## Fehlerklassen der LLM-Antworten (Repair CI/apfel)

Fehler beim Parsen von Architect-/Verify-Antworten sind seit der Repair-Phase
diagnostisch klassifiziert und erscheinen im `error.message`:

```text
empty response      — Provider lieferte keinen Inhalt
invalid JSON        — Inhalt ist kein JSON (Parse-Fehler)
truncated JSON      — JSON bricht vor dem Abschluss ab („… appears truncated
                      before valid JSON completion“, ggf. finish_reason=length)
schema violation    — valides JSON, aber falsche Struktur (kein Objekt)
```

Ein begrenzter Retry (max. 1 zusätzlicher Request, im Trace als Note +
`attempt: 2` sichtbar) erfolgt NUR bei reparablen Fehlern (invalid JSON,
schema violation). Bei Truncation/empty response gibt es KEINEN Retry — die
Ursache liegt im Output-Limit (siehe `LLM_MAX_TOKENS`), ein identischer
zweiter Call würde wieder kappen. Es wird niemals ein abgeschnittener JSON-
Output repariert oder als PASS gemeldet.
