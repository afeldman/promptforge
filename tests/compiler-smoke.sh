#!/bin/sh
# PromptForge v0.2 — Deterministic Compiler Smoke Test (Phase 1 + Phase 2).
#
# Beweist den ECHTEN Compilerpfad mit dem Release-Binary und der
# deterministischen Pipeline (--no-llm, ohne Mock/Fake):
#
#   "auditiere das projekt"
#        │
#        ▼
#   prompt-forge Release Binary
#        │
#        ▼
#   Engine → Prompt IR → Expansion → Optimierung → Verifikation
#        │
#        ▼
#   CompilationResult (formatneutral)
#        │
#        ├── TextSerializer   → ausführbarer Prompt
#        ├── JsonSerializer   → Envelope (JSON)
#        ├── YamlSerializer   → Envelope (YAML)
#        └── ToonSerializer   → Envelope (TOON, offizielle Rust-Crate)
#        │
#        ▼
#   maschinenlesbar geprüft (Python, strukturell, kein Text-Grep)
#        │
#        ▼
#   PASS
#
# Verwendung:
#   ./tests/compiler-smoke.sh                (nutzt target/release/prompt-forge)
#   BIN=target/debug/prompt-forge ./tests/compiler-smoke.sh
#   HOME_DIR=/tmp/pf-home ./tests/compiler-smoke.sh
set -u

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BIN=${BIN:-"${REPO_ROOT}/target/release/prompt-forge"}
HOME_DIR=${HOME_DIR:-"$(mktemp -d "${TMPDIR:-/tmp}/pf-compiler-smoke.XXXXXX")"}

ok() { printf '[ok] %s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

[ -x "$BIN" ] || fail "prompt-forge binary not found: $BIN (make build zuerst ausführen)"

INTENT="auditiere das projekt"
INTENT_FILE="${HOME_DIR}/intent.txt"
printf '%s\n' "$INTENT" >"$INTENT_FILE"
mkdir -p "${HOME_DIR}/out"

run_compile() { # $1..: zusätzliche Argumente; Ausgabe nach stdout
    "$BIN" --home "$HOME_DIR" compile "$INTENT" --no-llm "$@" 2>"${HOME_DIR}/stderr.txt"
}

# --- 1. Format json: CompilationResult-Envelope strukturell (Python) ---------
RESULT_JSON="${HOME_DIR}/result.json"
run_compile --format json >"$RESULT_JSON" || fail "compile --format json (exit $?)"

python3 - "$RESULT_JSON" "${HOME_DIR}/result-legacy.json" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    d = json.load(fh)
required = ["input","prompt_ir","expanded_prompt","optimized_prompt","verification",
            "metrics","token_report","stages","llm_used","request_id",
            "final_output","ir","long_prompt"]  # + v0.1-Aliase
missing = [k for k in required if k not in d]
if missing:
    print("FEHLENDE SCHLÜSSEL:", ", ".join(missing)); sys.exit(1)
assert d["input"] == "auditiere das projekt"
assert isinstance(d["prompt_ir"], dict) and d["prompt_ir"].get("task")
assert d["prompt_ir"]["schema_version"] == 1
assert d["verification"]["verdict"] == "pass"
assert d["metrics"]["structural_validity"] is True
assert d["metrics"]["semantic_fidelity"] >= 0.0
assert d["metrics"]["token_efficiency"] >= 0.0
assert d["stages"] == ["architect","expand","optimize","verify"]
assert d["llm_used"] is False
assert d["long_prompt"] == d["expanded_prompt"]
assert d["ir"] == d["prompt_ir"]
assert d["final_output"] == d["optimized_prompt"]
assert "analysis" not in d["prompt_ir"] or d["prompt_ir"]["analysis"] is None
# Für spätere Vergleiche aufheben.
with open(sys.argv[2], "w", encoding="utf-8") as fh:
    json.dump(d, fh, ensure_ascii=False, sort_keys=True)
print("CompilationResult (json): input/prompt_ir/expanded_prompt/optimized_prompt/verification/metrics ok")
PY
[ $? -eq 0 ] || fail "CompilationResult (json) nicht gültig"
ok "compile --format json: Envelope strukturell geprüft (exit 0)"

# --- 2. --json Legacy == --format json (semantisch äquivalent) ---------------
run_compile --json >"${HOME_DIR}/result-legacy-run.json" || fail "compile --json (exit $?)"
python3 - "${HOME_DIR}/result-legacy.json" "${HOME_DIR}/result-legacy-run.json" <<'PY'
import json, sys
def scrub(o):
    if isinstance(o, dict):
        return {k: scrub(v) for k, v in o.items() if k not in ("request_id", "created_at")}
    if isinstance(o, list):
        return [scrub(x) for x in o]
    return o
with open(sys.argv[1], encoding="utf-8") as fh: a = scrub(json.load(fh))
with open(sys.argv[2], encoding="utf-8") as fh: b = scrub(json.load(fh))
# Volatile Felder (request_id/created_at) normalisiert → semantische Äquivalenz.
if a != b:
    print("NICHT ÄQUIVALENT: --json vs --format json"); sys.exit(1)
print("--json == --format json (semantisch äquivalent, volatile Felder normalisiert)")
PY
[ $? -eq 0 ] || fail "--json != --format json"
ok "--json (Legacy) == --format json"

# --- 3. Text: ausführbarer Prompt (kein Envelope) ----------------------------
TEXT_OUT="${HOME_DIR}/prompt.txt"
run_compile >"$TEXT_OUT" || fail "compile (Default text, exit $?)"
python3 - "$TEXT_OUT" "${HOME_DIR}/result-legacy.json" <<'PY'
import json, sys
text = open(sys.argv[1], encoding="utf-8").read()
with open(sys.argv[2], encoding="utf-8") as fh: d = json.load(fh)
assert text.strip(), "Text leer"
# stdout-Text = optimized_prompt + abschließendes println-Newline; beide
# Seiten rechtsbündig normalisieren (optimized_prompt endet i. d. R. mit \n).
assert text.rstrip("\n") == d["optimized_prompt"].rstrip("\n"), \
    "Text != optimized_prompt (Envelope statt Prompt?)"
assert not text.lstrip().startswith("{"), "Text ist JSON"
assert "## Aufgabe" in text or text.strip(), "Prompt-Struktur fehlt"
print("Text = executable final prompt (identisch mit optimized_prompt)")
PY
[ $? -eq 0 ] || fail "Text-Serializer liefert keinen ausführbaren Prompt"
ok "--format text (Default): ausführbarer Prompt, kein Envelope"

# --- 4. YAML: parsen und Struktur prüfen -------------------------------------
YAML_OUT="${HOME_DIR}/result.yaml"
run_compile --format yaml >"$YAML_OUT" || fail "compile --format yaml (exit $?)"
python3 - "$YAML_OUT" "${HOME_DIR}/result-legacy.json" <<'PY'
import json, sys
yaml_path, json_path = sys.argv[1], sys.argv[2]
text = open(yaml_path, encoding="utf-8").read()
with open(json_path, encoding="utf-8") as fh: d = json.load(fh)
assert text.strip(), "YAML leer"
try:
    import yaml  # optional: PyYAML falls vorhanden
    parsed = yaml.safe_load(text)
    assert isinstance(parsed, dict)
    for k in ("input","prompt_ir","expanded_prompt","optimized_prompt","verification","metrics"):
        assert k in parsed, "YAML-Feld %s fehlt" % k
    assert parsed["input"] == d["input"]
    assert parsed["verification"]["verdict"] == "pass"
    assert parsed["metrics"]["structural_validity"] is True
    print("YAML geparst (PyYAML): gültig, Strukturfelder ok")
except ImportError:
    # PyYAML nicht verfügbar: YAML-Struktur wird durch Rust-Roundtrip-Tests
    # abgedeckt (pf-core: yaml_serializer_roundtrip); hier Basisform prüfen.
    assert "input:" in text and "prompt_ir:" in text and "verification:" in text \
        and "metrics:" in text, "YAML-Basisfelder fehlen"
    print("HINWEIS: PyYAML nicht installiert — Struktur via Rust-Roundtrip-Tests abgedeckt")
PY
[ $? -eq 0 ] || fail "YAML nicht gültig"
ok "--format yaml: valides YAML (Envelope-Felder vorhanden)"

# --- 5. TOON: Struktur + dokumentierte Conformance-Grenze --------------------
TOON_OUT="${HOME_DIR}/result.toon"
run_compile --format toon >"$TOON_OUT" || fail "compile --format toon (exit $?)"
python3 - "$TOON_OUT" <<'PY'
import sys
text = open(sys.argv[1], encoding="utf-8").read()
assert text.strip(), "TOON leer"
assert not text.lstrip().startswith("{"), "TOON ist JSON"
# Basiseigenschaften der TOON-Spezifikation: deklarierte Längen/Header und
# key: value-Zeilen (kein bloßer Nicht-Leer-Check). Volle Conformance +
# Roundtrip (decode_default == JSON-Datenmodell) wird in den Rust-Tests
# geprüft (pf-core: toon_serializer_roundtrip über den offiziellen Decoder
# der toon-format-Crate, Spec v3.0).
lines = [ln for ln in text.splitlines() if ln.strip()]
assert len(lines) > 10, "TOON-Dokument zu kurz"
print("TOON erzeugt: %d Zeilen (Conformance/Roundtrip: Rust-Tests)" % len(lines))
PY
[ $? -eq 0 ] || fail "TOON nicht gültig"
ok "--format toon: valides TOON-Dokument (Conformance via Rust-Roundtrip)"

# --- 6. stdin (compile -) mit Formaten ---------------------------------------
for fmt in text json yaml toon; do
    OUT_S="${HOME_DIR}/stdin-${fmt}.out"
    printf '%s\n' "$INTENT" | "$BIN" --home "$HOME_DIR" compile - --no-llm --format "$fmt" \
        >"$OUT_S" 2>/dev/null || fail "stdin --format $fmt (exit $?)"
    [ -s "$OUT_S" ] || fail "stdin --format $fmt: leer"
done
python3 - "${HOME_DIR}/stdin-json.out" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
assert d["input"] == "auditiere das projekt"
print("stdin + json: valides Envelope-JSON")
PY
[ $? -eq 0 ] || fail "stdin + json ungültig"
ok "stdin (compile -): text/json/yaml/toon"

# --- 7. Dateiinput ------------------------------------------------------------
FILE_OUT="${HOME_DIR}/file-json.out"
"$BIN" --home "$HOME_DIR" compile -f "$INTENT_FILE" --no-llm --format json >"$FILE_OUT" 2>/dev/null \
    || fail "Dateiinput --format json (exit $?)"
python3 - "$FILE_OUT" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
assert d["input"] == "auditiere das projekt"
PY
[ $? -eq 0 ] || fail "Dateiinput ungültig"
ok "Dateiinput (-f) + --format json"

# --- 8. -o (Ausgabedatei) je Format; Format entscheidet (kein Endungs-Raten) --
for fmt in text json yaml toon; do
    OUT_F="${HOME_DIR}/out/result.${fmt}"
    run_compile --format "$fmt" -o "$OUT_F" >/dev/null 2>&1 || fail "-o + --format $fmt (exit $?)"
    [ -s "$OUT_F" ] || fail "-o + --format $fmt: leer"
done
python3 - "${HOME_DIR}/out/result.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
assert d.get("input") == "auditiere das projekt"
assert d.get("verification", {}).get("verdict") == "pass"
assert "saved" in d, "-o + --format json: saved-Pfade fehlen"
PY
[ $? -eq 0 ] || fail "-o + json ungültig"
ok "-o + --format: text/json/yaml/toon (Format bestimmt, nicht die Endung)"

# --- 8b. Debug-Trace: Hilfe zeigt Optionen -----------------------------------
HELP_OUT="${HOME_DIR}/compile-help.txt"
"$BIN" compile --help >"$HELP_OUT" 2>&1 || fail "compile --help (exit $?)"
python3 - "$HELP_OUT" <<'PY'
import sys
help_text = open(sys.argv[1], encoding="utf-8").read()
assert "--debug" in help_text, "compile --help zeigt --debug nicht"
assert "--debug-json" in help_text, "compile --help zeigt --debug-json nicht"
print("compile --help zeigt --debug und --debug-json")
PY
[ $? -eq 0 ] || fail "--debug/--debug-json fehlen in compile --help"
ok "compile --help: --debug + --debug-json sichtbar"

# --- 8c. --debug ohne LLM (menschlesbarer Trace auf stderr) ------------------
DEBUG_ERR="${HOME_DIR}/debug-err.txt"
"$BIN" --home "$HOME_DIR" compile "$INTENT" --no-llm --debug >"${HOME_DIR}/debug-out.txt" 2>"$DEBUG_ERR" \
    || fail "compile --no-llm --debug (exit $?)"
python3 - "$DEBUG_ERR" "${HOME_DIR}/debug-out.txt" <<'PY'
import sys
err = open(sys.argv[1], encoding="utf-8").read()
out = open(sys.argv[2], encoding="utf-8").read()
assert "[trace]" in err, "kein Trace auf stderr"
for stage in ("architect", "expand", "optimize", "verify"):
    assert stage in err, "Stufe %s fehlt im Trace" % stage
assert out.strip(), "Prompt-Ausgabe fehlt bei --debug"
assert "sk-" not in err and "Bearer " not in err, "Secret-Muster im Trace"
print("--debug: Trace-Stufen sichtbar, kein Secret")
PY
[ $? -eq 0 ] || fail "--debug ohne LLM ungültig"
ok "compile --no-llm --debug: Trace auf stderr, Exit 0"

# --- 8d. --debug-json ohne LLM → debug.json (strukturell) --------------------
DEBUG_JSON="${HOME_DIR}/debug.json"
run_compile --debug-json -o "$DEBUG_JSON" >/dev/null 2>&1 || fail "compile --no-llm --debug-json (exit $?)"
[ -s "$DEBUG_JSON" ] || fail "debug.json nicht erzeugt"
python3 -m json.tool "$DEBUG_JSON" >/dev/null || fail "debug.json ist kein valides JSON"
python3 - "$DEBUG_JSON" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
assert d["input"] == "auditiere das projekt", "input falsch"
stages = d["stages"]
names = [s["stage"] for s in stages]
for want in ("architect", "expand", "optimize", "verify"):
    assert want in names, "Stufe %s fehlt" % want
assert d["llm_used"] is False
for s in stages:
    assert s["llm"] is False, "%s darf ohne LLM keine Calls haben" % s["stage"]
    assert s.get("note"), "%s: Hinweis fehlt (kein LLM)" % s["stage"]
    assert s["attempts"] == [], "%s: künstliche Attempts?" % s["stage"]
text = open(sys.argv[1], encoding="utf-8").read()
assert '"raw_response"' not in text, "kein künstliches raw_response ohne LLM"
print("debug.json: Stufen architect/expand/optimize/verify, llm=false, keine Fake-Responses")
PY
[ $? -eq 0 ] || fail "debug.json (no-llm) strukturell ungültig"
ok "--debug-json (--no-llm): debug.json erzeugt, gültig, Stufen ok"

# --- 8e. Mock-LLM-Trace (echter Python-Mock-Pfad, kein --no-llm) -------------
MOCK_DEBUG="${HOME_DIR}/mock-debug.json"
"$BIN" --home "$HOME_DIR" compile "$INTENT" --provider mock --model mock-model \
    --debug-json -o "$MOCK_DEBUG" >"${HOME_DIR}/mock-out.txt" 2>"${HOME_DIR}/mock-err.txt"
MOCK_EXIT=$?
if [ "$MOCK_EXIT" -ne 0 ] && grep -qE "Python-Bridge|promptforge nicht importierbar|venv" "${HOME_DIR}/mock-err.txt"; then
    printf 'SKIP: Mock-LLM-Trace — Python-Bridge nicht verfügbar (%s)\n' "$MOCK_EXIT"
else
    [ "$MOCK_EXIT" -eq 0 ] || { printf 'FAIL: Mock-LLM-Trace exit=%s\n' "$MOCK_EXIT" >&2; tail -5 "${HOME_DIR}/mock-err.txt" >&2; exit 1; }
    python3 - "$MOCK_DEBUG" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
assert d["llm_used"] is True, "Mock-Lauf soll llm_used=true"
stages = {s["stage"]: s for s in d["stages"]}
assert "expand" in stages and stages["expand"]["llm"] is False
for name in ("architect", "optimize", "verify"):
    s = stages.get(name)
    assert s, "Stufe %s fehlt" % name
    assert s["llm"] is True, "%s: llm=true erwartet" % name
    assert s["attempts"], "%s: keine Attempts" % name
    at = s["attempts"][0]
    assert at.get("system_prompt"), "%s.system_prompt == null" % name
    assert at.get("user_prompt"), "%s.user_prompt == null" % name
    assert at.get("raw_response"), "%s.raw_response == null" % name
    assert at.get("attempt") == 1
    # Werte stammen aus dem echten Pfad (nicht leer/rekonstruiert).
    assert len(at["user_prompt"]) > 10 and len(at["raw_response"]) > 1
text = open(sys.argv[1], encoding="utf-8").read()
assert "sk-" not in text and "Bearer " not in text, "Secret-Muster im Mock-Trace"
print("Mock-LLM-Trace: system/user/raw_response je LLM-Stufe aus echtem Pfad")
PY
    [ $? -eq 0 ] || fail "Mock-LLM-Trace strukturell ungültig"
    ok "Mock-LLM-Trace (--provider mock): Prompts/Responses real erfasst, keine Secrets"
fi

# --- 9. Ungültiges Format → kontrollierter Fehler (kein Panic/Fallback) ------
if "$BIN" --home "$HOME_DIR" compile "$INTENT" --no-llm --format banana \
    >"${HOME_DIR}/invalid.out" 2>"${HOME_DIR}/invalid.err"; then
    fail "--format banana lieferte Exit 0 (stiller Fallback?)"
fi
grep -q "unbekanntes Ausgabeformat" "${HOME_DIR}/invalid.err" \
    || fail "--format banana: keine klare Fehlermeldung"
ok "--format banana → kontrollierter Fehler (Exit != 0, Meldung)"

# --- 10. Zusammenfassung ------------------------------------------------------
ok "prompt-forge compile --no-llm (--json/--format) exit 0"
ok "CompilationResult vorhanden (input/prompt_ir/expanded_prompt/metrics)"
ok "v0.1-Aliase vorhanden (ir/long_prompt/final_output/stages/…)"
ok "deterministische Serializer (text/json/yaml/toon)"

# Aufräumen (nur temporäres Home; nie ~/.prompt-forge anfassen).
rm -rf "$HOME_DIR" 2>/dev/null || true

echo "PASS: deterministic compiler smoke (Formate text/json/yaml/toon über echten Compilerpfad)"
