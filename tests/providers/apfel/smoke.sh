#!/usr/bin/env bash
#
# smoke.sh — Reproduzierbarer PromptForge ↔ apfel Integrationstest (echter LLM-Pfad).
#
# Voraussetzungen (siehe README.md im selben Verzeichnis):
#   - macOS auf Apple Silicon, `apfel` installiert (keine Auto-Installation)
#   - Release-Binary vorhanden: `cargo build --release`
#     (vorher: export PYO3_PYTHON="$(pwd)/python/.venv/bin/python")
#   - python3 (macOS Command Line Tools)
#
# Modi:
#   ./tests/providers/apfel/smoke.sh          # startet eigenen apfel-Server
#   ./tests/providers/apfel/smoke.sh --existing  # nutzt laufenden Server, beendet ihn nicht
#
# Optional:
#   APFEL_ENDPOINT=http://127.0.0.1:11434/v1   # anderer Endpoint
#   APFEL_MODEL=<model-id>                     # Model explizit wählen
#
# Exit-Codes: 0 = PASS, 0 = SKIP (kein Fehler), 1 = FAIL.

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
BIN="${REPO_ROOT}/target/release/prompt-forge"
APFEL_ENDPOINT="${APFEL_ENDPOINT:-http://127.0.0.1:11434/v1}"
BASE_ROOT="${APFEL_ENDPOINT%/v1}"
INTENT="Erkläre die Grundidee von PromptForge. Beschreibe, warum die Pipeline aus Intent, Prompt IR, Expansion, Optimierung und Verification sinnvoll ist."

STARTED_SERVER=0
SERVER_PID=""
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/pf-apfel.XXXXXX")"

fail() { echo "ERROR: $*" >&2; exit 1; }
ok()   { echo "[ok] $*"; }

cleanup() {
    if [ "${STARTED_SERVER}" = "1" ] && [ -n "${SERVER_PID}" ]; then
        kill "${SERVER_PID}" 2>/dev/null || true
        wait "${SERVER_PID}" 2>/dev/null || true
        STARTED_SERVER=0
    fi
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT INT TERM

# --- HTTP-Helfer über python3 (kein curl/jq erforderlich) ---
# http_get <url>  -> Body auf stdout; Exit 2 bei Fehler/HTTP != 200
# http_post <url> <json-payload> -> Body auf stdout; Exit 2 bei Fehler/HTTP != 200
http_get() {
    python3 - "$1" <<'PY'
import sys, urllib.request, urllib.error
url = sys.argv[1]
try:
    with urllib.request.urlopen(url, timeout=10) as r:
        sys.stdout.write(r.read().decode("utf-8", "replace"))
except urllib.error.HTTPError as e:
    sys.stderr.write("HTTP %s %s\n" % (e.code, url))
    sys.exit(2)
except Exception as e:
    sys.stderr.write("ERR %s %s\n" % (type(e).__name__, url))
    sys.exit(2)
PY
}

http_post() {
    python3 - "$1" "$2" <<'PY'
import sys, urllib.request, urllib.error
url, payload = sys.argv[1], sys.argv[2].encode("utf-8")
req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
try:
    with urllib.request.urlopen(req, timeout=180) as r:
        sys.stdout.write(r.read().decode("utf-8", "replace"))
except urllib.error.HTTPError as e:
    body = e.read().decode("utf-8", "replace")[:500]
    sys.stderr.write("HTTP %s %s body=%s\n" % (e.code, url, body))
    sys.exit(2)
except Exception as e:
    sys.stderr.write("ERR %s %s\n" % (type(e).__name__, url))
    sys.exit(2)
PY
}

# --- 1. Plattform-Checks (Skip ist KEIN Fehler) ---
OS_NAME="$(uname -s)"
ARCH="$(uname -m)"
if [ "${OS_NAME}" != "Darwin" ]; then
    echo "SKIP: apfel integration requires macOS (current: ${OS_NAME})"
    exit 0
fi
if [ "${ARCH}" != "arm64" ]; then
    echo "SKIP: apfel integration requires Apple Silicon (current: ${ARCH})"
    exit 0
fi
ok "macOS ${OS_NAME} / Apple Silicon (${ARCH})"

if ! command -v apfel >/dev/null 2>&1; then
    echo "SKIP: apfel not installed — siehe tests/providers/apfel/README.md"
    exit 0
fi
APFEL_VERSION="$(apfel --version 2>/dev/null | head -1)"
ok "apfel: ${APFEL_VERSION:-unbekannte Version}"

# --- 2. Release-Binary (Test eines gebauten Builds — kein Auto-Build) ---
if [ ! -x "${BIN}" ]; then
    fail "prompt-forge release binary not found: ${BIN}
Run: export PYO3_PYTHON=\"\$(pwd)/python/.venv/bin/python\" && cargo build --release"
fi
ok "release binary: ${BIN}"

# --- 3. Server-Lifecycle ---
MODE="default"
if [ "${1:-}" = "--existing" ]; then
    MODE="existing"
elif [ $# -gt 0 ]; then
    fail "unbekanntes Argument: $1 (erlaubt: --existing)"
fi

if [ "${MODE}" = "existing" ]; then
    ok "Modus: --existing (laufender apfel-Server wird genutzt, nicht beendet)"
elif health_body="$(http_get "${BASE_ROOT}/health" 2>/dev/null)"; then
    # Bereits ein Server erreichbar (z. B. via make apfel-start): keinen
    # zweiten Server starten, am Ende auch nicht beenden.
    ok "apfel läuft bereits (${BASE_ROOT}/health) — kein zweiter Server"
else
    ok "Starte apfel --serve (Hintergrund) …"
    apfel --serve >"${TMP_DIR}/apfel-server.log" 2>&1 &
    SERVER_PID=$!
    STARTED_SERVER=1
fi

# --- 4. Bereitschaft / Health ---
ready=0
for _ in $(seq 1 90); do
    if health_body="$(http_get "${BASE_ROOT}/health" 2>/dev/null)"; then
        ready=1
        break
    fi
    if models_body="$(http_get "${APFEL_ENDPOINT}/models" 2>/dev/null)"; then
        ready=1
        break
    fi
    sleep 1
done
if [ "${ready}" != "1" ]; then
    if [ "${MODE}" = "default" ]; then
        echo "--- apfel server log (Auszug) ---" >&2
        tail -20 "${TMP_DIR}/apfel-server.log" >&2 || true
    fi
    fail "apfel server did not become ready at ${APFEL_ENDPOINT}"
fi
ok "health verfügbar (${BASE_ROOT}/health)"

# --- 5. Model-Discovery (nicht hartcodiert) ---
models_body="$(http_get "${APFEL_ENDPOINT}/models")" || fail "/v1/models failed"
MODEL="$(printf '%s' "${models_body}" | MODEL_OVERRIDE="${APFEL_MODEL:-}" python3 -c '
import json, os, sys
d = json.load(sys.stdin)
ids = [m.get("id") for m in d.get("data", []) if m.get("id")]
override = os.environ.get("MODEL_OVERRIDE", "")
if override:
    print(override); sys.exit(0)
if not ids:
    print(""); sys.exit(1)
print(next((i for i in ids if i == "apple-foundationmodel"), ids[0]))
')" || fail "no model discovered in /v1/models"
[ -n "${MODEL}" ] || fail "no model discovered"
ok "/v1/models erfolgreich"
ok "Model: ${MODEL}"

# --- 6. Direkter OpenAI-Kompatibilitätstest ---
chat_body="$(http_post "${APFEL_ENDPOINT}/chat/completions" "{\"model\":\"${MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"Antworte exakt mit: APFEL_OK\"}],\"max_tokens\":20}")" \
    || fail "chat completion failed"
printf '%s' "${chat_body}" | python3 -c '
import json, sys
d = json.load(sys.stdin)
choices = d.get("choices") or []
msg = (choices[0].get("message") or {}) if choices else {}
content = msg.get("content")
assert content is not None and str(content).strip() != "", "leere assistant message"
print("Antwort:", str(content).strip()[:120])
' || fail "chat completion failed (Antwort nicht OpenAI-kompatibel)"
ok "chat completion erfolgreich"

# --- 7. PromptForge Environment (nur Testprozess; keine Keys) ---
export LLM_ENDPOINT="${APFEL_ENDPOINT}"
export LLM_MODEL="${MODEL}"
export LLM_KEY=""          # apfel benötigt keinen Key; ererbte Keys neutralisieren
# Output-Limit explizit setzen: Apple Foundation Model kappt Antworten ohne
# max_tokens am Server-Default (Root Cause der Architect-Truncation).
# 3072 bleibt unter dem 4096er-Kontext und erlaubt vollständige IRs.
export LLM_MAX_TOKENS="${LLM_MAX_TOKENS:-3072}"
unset PF_PROVIDER
ok "LLM_MAX_TOKENS=${LLM_MAX_TOKENS} (explizites Output-Limit gegen Truncation)"

# --- 8. Echter PromptForge-Durchlauf (kein Mock, kein --no-llm) ---
START_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
"${BIN}" compile --json "${INTENT}" >"${TMP_DIR}/result.json" 2>"${TMP_DIR}/result.err"
RC=$?
END_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
LATENCY_MS=$((END_MS - START_MS))

if [ "${RC}" -ne 0 ]; then
    echo "--- prompt-forge stderr ---" >&2
    cat "${TMP_DIR}/result.err" >&2
    fail "prompt-forge returned exit code ${RC} (${LATENCY_MS} ms)"
fi

# --- 9. Ergebnis prüfen & Zusammenfassung ---
python3 - "${TMP_DIR}/result.json" "${LATENCY_MS}" "${MODEL}" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
latency = int(sys.argv[2])
model = sys.argv[3]
tr = d.get("token_report") or {}
v = d.get("verification") or {}
stages = d.get("stages") or []
m = d.get("metrics") or {}
checks = {
    "llm_used": bool(d.get("llm_used")),
    "architect": "architect" in stages,
    "expand": "expand" in stages,
    "optimize": "optimize" in stages,
    "verify": "verify" in stages,
    "optimized_prompt": bool(d.get("optimized_prompt")),
    "verdict_pass": v.get("verdict") == "pass",
    # v0.2: CompilationResult-Struktur maschinenlesbar prüfen.
    "input": bool(d.get("input")),
    "prompt_ir": bool(d.get("prompt_ir")),
    "expanded_prompt": bool(d.get("expanded_prompt")),
    "final_output": bool(d.get("final_output")),
    "metrics": "semantic_fidelity" in m and "token_efficiency" in m and "structural_validity" in m,
    "compilation_result_legacy": bool(d.get("ir")) and bool(d.get("long_prompt")),
}
missing = [k for k, val in checks.items() if not val]
if missing:
    print("ERFOLGSKRITERIEN FEHLEN:", ", ".join(missing))
    sys.exit(3)

original = int(tr.get("original") or 0)
generated = int(tr.get("generated") or 0)
optimized = int(tr.get("optimized") or 0)
reduction = (1.0 - optimized / generated) * 100.0 if generated else 0.0
print("PromptForge / apfel integration")
print("-" * 44)
print("Model: %s" % model)
print("Input: %d estimated tokens" % original)
print("Expanded: %d estimated tokens" % generated)
print("Optimized: %d estimated tokens" % optimized)
print("Reduction: %.1f%% estimated" % reduction)
print("Verification: %s (semantic %.2f)" % (v.get("verdict"), float(v.get("semantic_preservation") or 0.0)))
print("Latency: %d ms" % latency)
print("Status: PASS")
PY
[ $? -eq 0 ] || fail "PromptForge-Ergebnis nicht gültig (siehe oben)"

ok "LLM Call erfolgreich"
ok "Architect erfolgreich"
ok "Prompt Expansion erfolgreich"
ok "Optimization erfolgreich"
ok "Verification erfolgreich"
ok "CompilationResult vorhanden (input/prompt_ir/expanded_prompt/metrics)"
ok "finaler Prompt vorhanden"

# --- 10. v1.0 Optimization-Engine: echter Benchmark (realer LLM-Pfad) ---
# Mehrere Intents (Kurz/Deutsch + technisch/englisch + langer technischer
# Prompt mit Constraints, Code/CLI, Pfaden, URL). Kein Mock.
OPT_INTENTS=(
    "auditiere das projekt"
    "Audit the project architecture, identify security risks, check dependency management, verify test coverage and produce an actionable report without modifying source files."
    "Prüfe das Repository unter /Users/dev/probe auf Sicherheitsprobleme. Führe cargo audit und cargo deny check aus, bewerte die CI-Workflow-Datei .github/workflows/ci.yml, prüfe Abhängigkeiten gegen https://osv.dev und erstelle einen Bericht nach docs/report.md. Ändere keine Quelldateien und keine Konfigurationen."
)

opt_bad=0
i=0
for intent in "${OPT_INTENTS[@]}"; do
    i=$((i + 1))
    out_json="${TMP_DIR}/opt-${i}.json"
    err_file="${TMP_DIR}/opt-${i}.err"
    START_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
    "${BIN}" compile --json "${intent}" >"${out_json}" 2>"${err_file}"
    rc=$?
    END_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
    lat=$((END_MS - START_MS))
    if [ "${rc}" -ne 0 ]; then
        echo "--- optimizer benchmark intent ${i}: stderr ---" >&2
        cat "${err_file}" >&2
        echo "FAIL: optimizer benchmark intent ${i} (exit ${rc})" >&2
        opt_bad=1
        continue
    fi
    python3 - "${out_json}" "${i}" "${lat}" "${MODEL}" <<'PY' || opt_bad=1
import json, sys
d = json.load(open(sys.argv[1]))
i, lat, model = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
o = d.get("optimization") or {}
cands = o.get("candidates") or []
tr = d.get("token_report") or {}
v = d.get("verification") or {}
gen = int(tr.get("generated") or 0)
out = int(tr.get("optimized") or 0)
red = (1.0 - out / gen) * 100.0 if gen else 0.0
problems = []
if o.get("optimization_status") not in ("optimized", "no_improvement"):
    problems.append("status=%s" % o.get("optimization_status"))
if not cands:
    problems.append("keine Kandidaten")
if not any(c.get("strategy") == "structural" for c in cands):
    problems.append("structural-Kandidat fehlt")
if not any(c.get("verification") == "pass" for c in cands):
    problems.append("kein passender Kandidat")
if not o.get("selected") and o.get("optimization_status") == "optimized":
    problems.append("selected fehlt bei optimized")
if v.get("verdict") != "pass":
    problems.append("verdict=%s" % v.get("verdict"))
m = d.get("metrics") or {}
if "technical_token_preservation" not in m:
    problems.append("technical_token_preservation fehlt")
if problems:
    print("INTENT %d: PROBLEME %s" % (i, ", ".join(problems)))
    sys.exit(1)
sel = o.get("selected") or "-"
print("INTENT %d | Model %s | gen=%d → out=%d (Reduktion %.1f%%) | status=%s selected=%s | semantic %.2f | technical %.2f | %d Kandidaten | %d ms"
      % (i, model, gen, out, red, o.get("optimization_status"), sel,
         float(v.get("semantic_preservation") or 0.0),
         float(m.get("technical_token_preservation") or 1.0),
         len(cands), lat))
PY
done
if [ "${opt_bad}" -ne 0 ]; then
    fail "Optimization-Benchmark nicht bestanden (siehe oben; echte LLM-Ausgaben, kein Mock)"
fi
ok "Optimization-Engine: Benchmark über ${i} Intents erfolgreich (echte apfel-Calls)"

echo "PASS: PromptForge ↔ apfel integration (inkl. v1.0 Optimization Engine)"
