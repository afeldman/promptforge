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
unset PF_PROVIDER

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
checks = {
    "llm_used": bool(d.get("llm_used")),
    "architect": "architect" in stages,
    "expand": "expand" in stages,
    "optimize": "optimize" in stages,
    "verify": "verify" in stages,
    "optimized_prompt": bool(d.get("optimized_prompt")),
    "verdict_pass": v.get("verdict") == "pass",
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
ok "finaler Prompt vorhanden"
echo "PASS: PromptForge ↔ apfel integration"
