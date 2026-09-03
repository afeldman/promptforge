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
