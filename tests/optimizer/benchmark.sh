#!/usr/bin/env bash
#
# optimizer-benchmark.sh — Deterministischer Optimizer-Benchmark (kein LLM).
#
# Vergleicht die v1.0-Optimization-Engine (Strategien auto/baseline/
# redundancy/structural/semantic/combined) gegen den bisherigen Optimizer
# (baseline = Long Prompt, vor v1.0 das einzige no-llm-Ergebnis).
#
# Alle Zahlen sind real gemessene Heuristik-Tokens (estimated) aus dem
# Release-Binary — keine simulierten Werte. Real-LLM-Benchmark (apfel)
# läuft separat über `tests/providers/apfel/smoke.sh` / `make test-apfel`.
#
# Verwendung:
#   export PYO3_PYTHON="$(pwd)/python/.venv/bin/python"
#   cargo build --release
#   ./tests/optimizer/benchmark.sh
#
# Exit: 0 = PASS, 1 = FAIL (Engine-Verletzung: Erhaltung/Nicht-Verschlechterung).

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BIN="${REPO_ROOT}/target/release/prompt-forge"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/pf-optbench.XXXXXX")"
trap 'rm -rf "${TMP_DIR}"' EXIT

if [ ! -x "${BIN}" ]; then
    echo "FAIL: release binary not found: ${BIN}"
    echo "Run: export PYO3_PYTHON=\"\$(pwd)/python/.venv/bin/python\" && cargo build --release"
    exit 1
fi

# Realistische Intent-Matrix (deutsch/englisch, kurz/technisch/lang).
INTENTS=(
    "auditiere das projekt"
    "Analysiere diese fünf Papers und vergleiche die Methoden"
    "Audit the project architecture, identify security risks, check dependency management and verify test coverage without modifying source files."
    "Prüfe das Repository unter /Users/dev/probe. Führe cargo audit und cargo deny check aus, bewerte .github/workflows/ci.yml, prüfe Abhängigkeiten gegen https://osv.dev, erstelle docs/report.md. Ändere keine Quelldateien."
)

MODES=(baseline redundancy instruction structural semantic combined auto)

fail=0
echo "Optimizer-Benchmark (deterministisch, --no-llm, estimated tokens)"
echo "Engine: baseline = bisheriger no-llm-Optimizer (1:1 Long Prompt)"
echo "Strategien: redundancy | instruction | structural | semantic | combined | auto"
echo ""
printf "%-13s %8s %8s %8s %8s %8s %8s %s\n" \
    "intent" "mode" "gen" "out" "reduk%" "sem" "tech%" "guard"
i=0
for intent in "${INTENTS[@]}"; do
    i=$((i + 1))
    for mode in "${MODES[@]}"; do
        out_json="${TMP_DIR}/r-${i}-${mode}.json"
        if ! "${BIN}" compile --no-llm --optimizer "${mode}" --json "${intent}" >"${out_json}" 2>"${TMP_DIR}/e.log"; then
            echo "FAIL intent ${i} mode ${mode}: exit ${?} (siehe ${TMP_DIR}/e.log)"
            fail=1
            continue
        fi
        python3 - "${out_json}" "${i}" "${mode}" <<'PY' || fail=1
import json, sys
d = json.load(open(sys.argv[1]))
i = int(sys.argv[2])
mode = sys.argv[3]
o = d.get("optimization") or {}
tr = d.get("token_report") or {}
v = d.get("verification") or {}
m = d.get("metrics") or {}
gen = int(tr.get("generated") or 0)
out = int(tr.get("optimized") or 0)
red = (1.0 - out / gen) * 100.0 if gen else 0.0
# Engine-Verletzungen: nie größer als Long Prompt, nie Information verlieren.
problems = []
if o.get("optimization_status") not in ("optimized", "no_improvement"):
    problems.append("status")
if out > gen:
    problems.append("out>gen")
if v.get("verdict") != "pass":
    problems.append("verdict")
if float(m.get("technical_token_preservation") or 1.0) < 0.99:
    problems.append("tech<0.99")
if problems:
    print("FAIL intent %d mode %s: %s" % (i, mode, ", ".join(problems)))
    sys.exit(1)
status = o.get("optimization_status") or "-"
sel = o.get("selected") or ""
sel_s = ("/" + sel) if sel else ""
print("%-13s %8s %8d %8d %7.1f%% %7.2f %7.2f  %s%s"
      % ("intent%d" % i, mode, gen, out, red,
         float(v.get("semantic_preservation") or 0.0),
         float(m.get("technical_token_preservation") or 1.0),
         status, sel_s))
PY
    done
done

echo ""
if [ "${fail}" -ne 0 ]; then
    echo "PASS WITH FAILURES? → FAIL (siehe oben)"
    exit 1
fi
echo "PASS: alle Strategien erhaltungssicher; keine Verschlechterung; deterministisch"
exit 0
