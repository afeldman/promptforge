#!/usr/bin/env bash
#
# scripts/apfel.sh — Apfel-Lifecycle für die lokale macOS-Entwicklung.
#
# Wird von den Makefile-Targets apfel-start / apfel-stop / apfel-status
# verwendet. Erzeugt KEINE Runtime-Dateien im Repository: PID/Logs liegen
# unter APFEL_RUNTIME (Default: ~/.prompt-forge/state/apfel).
#
# Aufruf:
#   APFEL_ENDPOINT=... APFEL_RUNTIME=... scripts/apfel.sh start|stop|status|health
#
# Umgebungsvariablen:
#   APFEL_ENDPOINT   OpenAI-kompatibler Endpoint (Default http://127.0.0.1:11434/v1)
#   APFEL_RUNTIME    Runtime-Verzeichnis für PID/Logs (Default ~/.prompt-forge/state/apfel)
#   APFEL_TIMEOUT_S  Start-Timeout (Default 60)
set -u

APFEL_ENDPOINT="${APFEL_ENDPOINT:-http://127.0.0.1:11434/v1}"
BASE_ROOT="${APFEL_ENDPOINT%/v1}"
BASE_ROOT="${BASE_ROOT%/}"
APFEL_RUNTIME="${APFEL_RUNTIME:-${HOME}/.prompt-forge/state/apfel}"
APFEL_TIMEOUT_S="${APFEL_TIMEOUT_S:-60}"
PID_FILE="${APFEL_RUNTIME}/apfel.pid"
LOG_FILE="${APFEL_RUNTIME}/apfel-serve.log"

http_get() { # <url> → Body/exit 0 bei 200
    python3 - "$1" <<'PY'
import sys, urllib.request, urllib.error
try:
    with urllib.request.urlopen(sys.argv[1], timeout=5) as r:
        sys.stdout.write(r.read().decode("utf-8", "replace"))
except Exception:
    sys.exit(1)
PY
}

running_pid() {
    [ -f "${PID_FILE}" ] || { printf ''; return 1; }
    local pid
    pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
    case "${pid}" in
        ''|*[!0-9]*) rm -f "${PID_FILE}"; return 1 ;;
    esac
    if kill -0 "${pid}" 2>/dev/null; then
        printf '%s' "${pid}"
        return 0
    fi
    # verwaiste PID-Datei (Prozess beendet) aufräumen
    rm -f "${PID_FILE}"
    return 1
}

health_ok() {
    http_get "${BASE_ROOT}/health" >/dev/null 2>&1
}

cmd_start() {
    # Nur auf macOS/Apple Silicon; Plattform-Prüfung macht das Makefile.
    command -v apfel >/dev/null 2>&1 || {
        echo "ERROR: apfel is not installed. Run: brew install apfel" >&2
        exit 1
    }
    mkdir -p "${APFEL_RUNTIME}"

    if pid="$(running_pid)"; then
        if health_ok; then
            echo "apfel already running (pid ${pid}) — kein zweiter Server"
            exit 0
        fi
        echo "WARNING: apfel pid ${pid} läuft, aber /health antwortet nicht — Server wird beendet und neu gestartet" >&2
        kill "${pid}" 2>/dev/null || true
    fi

    apfel --serve >"${LOG_FILE}" 2>&1 &
    local pid=$!
    printf '%s\n' "${pid}" >"${PID_FILE}"

    local waited=0
    while [ "${waited}" -lt "${APFEL_TIMEOUT_S}" ]; do
        if health_ok; then
            echo "apfel started (pid ${pid}) — ${BASE_ROOT}/health ok"
            exit 0
        fi
        sleep 1
        waited=$((waited + 1))
    done

    # Timeout: aufräumen (kein verwaister Server)
    echo "ERROR: apfel did not become ready within ${APFEL_TIMEOUT_S}s (${BASE_ROOT}/health)" >&2
    tail -20 "${LOG_FILE}" >&2 || true
    kill "${pid}" 2>/dev/null || true
    wait "${pid}" 2>/dev/null || true
    rm -f "${PID_FILE}"
    exit 1
}

cmd_stop() {
    if ! pid="$(running_pid)"; then
        echo "apfel nicht von apfel-start verwaltet oder bereits beendet — nichts zu tun"
        exit 0
    fi
    # Nur den verwalteten Prozess beenden (PID aus eigener PID-Datei).
    kill "${pid}" 2>/dev/null || true
    local waited=0
    while kill -0 "${pid}" 2>/dev/null && [ "${waited}" -lt 10 ]; do
        sleep 1
        waited=$((waited + 1))
    done
    if kill -0 "${pid}" 2>/dev/null; then
        kill -9 "${pid}" 2>/dev/null || true
    fi
    wait "${pid}" 2>/dev/null || true
    rm -f "${PID_FILE}"
    echo "apfel stopped (pid ${pid})"
}

cmd_status() {
    if command -v apfel >/dev/null 2>&1; then
        echo "apfel installiert:  ja ($(apfel --version 2>/dev/null | head -1))"
    else
        echo "apfel installiert:  nein"
    fi
    if pid="$(running_pid)"; then
        echo "apfel läuft:        ja (pid ${pid})"
    else
        echo "apfel läuft:        nein"
    fi
    if health_ok; then
        echo "health:             ok (${BASE_ROOT}/health)"
        local models
        models="$(http_get "${APFEL_ENDPOINT}/models" 2>/dev/null || true)"
        if [ -n "${models}" ]; then
            printf '%s' "${models}" | APFEL_MODEL="${APFEL_MODEL:-}" python3 -c '
import json, os, sys
try:
    d = json.load(sys.stdin)
    ids = [m.get("id") for m in d.get("data", []) if m.get("id")]
    override = os.environ.get("APFEL_MODEL", "")
    print("model(s):", ", ".join([override] if override else ids[:3]) or "-")
except Exception:
    pass
' 2>/dev/null || true
        fi
    else
        echo "health:             nicht erreichbar (${BASE_ROOT}/health)"
    fi
    echo "endpoint:           ${APFEL_ENDPOINT}"
    echo "pid-file:           ${PID_FILE}"
}

cmd_health() {
    if health_ok; then
        echo "ok"
        exit 0
    fi
    echo "unavailable"
    exit 1
}

case "${1:-}" in
    start)  cmd_start ;;
    stop)   cmd_stop ;;
    status) cmd_status ;;
    health) cmd_health ;;
    *) echo "usage: $0 start|stop|status|health" >&2; exit 2 ;;
esac
