"""Operationen (Architect/Optimize/Verify) + Bridge-Einstieg (Rust→Python).

`handle_request(request: str) -> str` ist der JSON-Vertrag über PyO3."""

import json
import os
import time

from . import __version__, logging as pylog
from .errors import PromptForgeError
from .llm import provider as llm_provider
from .llm.mock import IR_SCHEMA, mock_complete
from .prompts import ARCHITECT_SYSTEM, OPTIMIZE_SYSTEM, VERIFY_SYSTEM


def _configure_logging_once():
    """Konfiguriert die Python-Logseite aus der Umgebung (idempotent).

    Datei-Sink nur, wenn PF_HOME explizit gesetzt ist; stderr nur bei
    PF_LOG_STDERR=1. Default: still (Rust-Seite loggt strukturiert).
    """
    home = os.environ.get("PF_HOME")
    log_dir = os.path.join(home, "logs") if home else None
    pylog.configure(
        level=os.environ.get("PF_LOG_LEVEL", "INFO"),
        fmt=os.environ.get("PF_LOG_FORMAT", "text"),
        log_dir=log_dir,
        to_stderr=os.environ.get("PF_LOG_STDERR") == "1",
    )


_configure_logging_once()


# --- Hilfsfunktionen ---

def _json_clean(text: str) -> str:
    """Entfernt ```json-Fences und trimmt."""
    t = text.strip()
    if t.startswith("```"):
        t = t.split("\n", 1)[1] if "\n" in t else t[3:]
        if t.endswith("```"):
            t = t[:-3]
        t = t.strip()
    return t


def _json_parse_failure(op: str, raw: str, resp: dict, exc: Exception) -> PromptForgeError:
    """Klassifiziert einen fehlgeschlagenen JSON-Parse eines LLM-Outputs.

    Unterscheidung (Repair CI/apfel): empty response | invalid JSON |
    truncated JSON | (schema violations werden nach erfolgreichem Parse
    separat geprüft). Ein abgeschnittener JSON-Output wird NICHT repariert
    oder geraten — er wird diagnostisch gemeldet; der vollständige Rohtext
    bleibt über --debug-json verfügbar (redigiert).
    """
    stripped = str(raw or "").strip()
    finish = str(resp.get("finish_reason") or "").strip().lower()
    if not stripped:
        return PromptForgeError(
            "model", f"{op}: empty response (kein Inhalt) — der Provider lieferte keine Antwort"
        )
    tail = stripped[-1]
    length_cut = finish == "length"
    open_json = stripped.lstrip().startswith("{") or stripped.lstrip().startswith("[")
    looks_truncated = length_cut or (
        open_json and not stripped.endswith(("}", "]", '"', "true", "false", "null"))
    )
    if looks_truncated:
        detail = "finish_reason=length" if length_cut else f"letztes Zeichen {tail!r}"
        return PromptForgeError(
            "model",
            f"{op} response appears truncated before valid JSON completion "
            f"({detail}); Antwort beginnt: {stripped[:160]!r} … endet: {stripped[-80:]!r}",
        )
    return PromptForgeError(
        "model",
        f"{op}: invalid JSON ({exc}); Antwort beginnt: {stripped[:160]!r} … endet: {stripped[-80:]!r}",
    )


def _req_json(request: str) -> dict:
    try:
        req = json.loads(request)
    except Exception as exc:
        raise PromptForgeError("bridge", f"Request ist kein valides JSON: {exc}") from exc
    if not isinstance(req, dict):
        raise PromptForgeError("bridge", "Request muss ein JSON-Objekt sein")
    return req


def _common(req: dict) -> dict:
    return {
        "endpoint": req.get("endpoint"),
        "api_key": req.get("api_key"),
        "model": req.get("model"),
        "temperature": req.get("temperature"),
        "max_tokens": req.get("max_tokens"),
        "timeout_s": int(req.get("timeout_s") or 120),
    }


def _call(op: str, req: dict, system: str | None, user_text: str) -> dict:
    """Führt einen LLM-Call aus (Mock oder any-llm) und liefert content/usages."""
    provider_kind = (req.get("provider") or "auto").strip().lower()
    started = time.monotonic()
    if provider_kind == "mock":
        resp = mock_complete(op, user_text)
    else:
        messages = []
        if system:
            messages.append({"role": "system", "content": system})
        messages.append({"role": "user", "content": user_text})
        kwargs = _common(req)
        kwargs["provider"] = provider_kind
        resp = llm_provider.chat(messages, **kwargs)
    resp["duration_ms"] = int((time.monotonic() - started) * 1000)
    return resp


def _ok(content: str, resp: dict, system_prompt=None, user_prompt=None) -> dict:
    """Baut die Antwort; optional mit Echo der tatsächlich verwendeten
    Prompt-Texte (Debug-Trace: System- und User-Prompt, wie gesendet)."""
    out = {"ok": True, "content": content}
    if system_prompt is not None:
        out["system_prompt"] = system_prompt
    if user_prompt is not None:
        out["user_prompt"] = user_prompt
    usage = resp.get("usage")
    if usage and (usage.get("prompt_tokens") is not None or usage.get("completion_tokens") is not None):
        out["usage"] = {
            "prompt_tokens": int(usage.get("prompt_tokens") or 0),
            "completion_tokens": int(usage.get("completion_tokens") or 0),
        }
    if resp.get("model"):
        out["model"] = resp["model"]
    if resp.get("finish_reason"):
        out["finish_reason"] = resp["finish_reason"]
    out["duration_ms"] = resp.get("duration_ms")
    return out


# --- Operationen ---

def _as_str_or_null(v) -> str | None:
    if v is None:
        return None
    if isinstance(v, str):
        s = v.strip()
        return s or None
    # Kleine Modelle liefern manchmal Objekte/Listen statt Strings.
    return str(v)[:500]


def _as_str_list(v) -> list:
    if isinstance(v, list):
        out = []
        for item in v:
            if isinstance(item, str):
                out.append(item.strip())
            elif item is not None:
                out.append(str(item)[:500])
        return out
    if isinstance(v, str):
        return [v.strip()] if v.strip() else []
    return []


def _normalize_ir(parsed: dict, request_id: str) -> dict:
    """Füllt fehlende IR-Schlüssel und erzwingt Typen (kleine On-Device-Modelle
    liefern oft unvollständiges oder typ-inhomogenes JSON)."""
    base = json.loads(json.dumps(IR_SCHEMA))
    base.update(parsed)
    base["schema_version"] = 1
    base["task"] = str(base.get("task") or "").strip()
    base["objective"] = _as_str_list(base.get("objective"))
    base["context"] = _as_str_list(base.get("context"))
    base["assumptions"] = _as_str_list(base.get("assumptions"))
    base["procedure"] = _as_str_list(base.get("procedure"))
    base["verification_requirements"] = _as_str_list(base.get("verification_requirements"))
    base["role"] = _as_str_or_null(base.get("role"))
    base["reasoning_strategy"] = _as_str_or_null(base.get("reasoning_strategy"))
    base["target_model"] = _as_str_or_null(base.get("target_model"))

    # inputs: [{name, description}]
    inputs = []
    if isinstance(base.get("inputs"), list):
        for item in base["inputs"]:
            if isinstance(item, dict):
                inputs.append(
                    {"name": _as_str_or_null(item.get("name")) or "", "description": _as_str_or_null(item.get("description")) or ""}
                )
    base["inputs"] = inputs

    # constraints: [{text, severity}]
    constraints = []
    if isinstance(base.get("constraints"), list):
        for item in base["constraints"]:
            if isinstance(item, dict):
                sev = str(item.get("severity") or "").lower()
                constraints.append(
                    {"text": _as_str_or_null(item.get("text")) or "", "severity": sev if sev in ("required", "recommended") else "required"}
                )
    base["constraints"] = constraints

    # examples: [{input, output}]
    examples = []
    if isinstance(base.get("examples"), list):
        for item in base["examples"]:
            if isinstance(item, dict):
                examples.append(
                    {"input": _as_str_or_null(item.get("input")) or "", "output": _as_str_or_null(item.get("output")) or ""}
                )
    base["examples"] = examples

    # output_contract
    oc = base.get("output_contract")
    if not isinstance(oc, dict):
        oc = {}
    oc = {
        "format": _as_str_or_null(oc.get("format")) or "markdown",
        "structure": _as_str_list(oc.get("structure")),
        "rules": _as_str_list(oc.get("rules")),
        "example": _as_str_or_null(oc.get("example")),
    }
    base["output_contract"] = oc

    base["metadata"] = {
        "request_id": request_id,
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source_language": None,
        "tags": _as_str_list(parsed.get("metadata", {}).get("tags")) if isinstance(parsed.get("metadata"), dict) else [],
        "engine_version": __version__,
    }
    return base


def op_architect(req: dict) -> dict:
    intent = req.get("user_prompt") or ""
    try:
        payload = json.loads(intent)
        if isinstance(payload, dict):
            intent = payload.get("intent") if payload else ""
    except Exception:
        pass
    if not str(intent).strip():
        raise PromptForgeError("invalid_input", "Intent ist leer")
    provider_kind = (req.get("provider") or "auto").strip().lower()
    if provider_kind == "mock":
        resp = mock_complete("architect", json.dumps({"intent": str(intent)}, ensure_ascii=False))
    else:
        messages = [
            {"role": "system", "content": ARCHITECT_SYSTEM},
            {"role": "user", "content": f"INTENT: {str(intent).strip()}"},
        ]
        kwargs = _common(req)
        kwargs["provider"] = provider_kind
        resp = llm_provider.chat(messages, **kwargs)
    raw = _json_clean(str(resp.get("content", "")))
    try:
        parsed = json.loads(raw)
    except Exception as exc:
        raise _json_parse_failure("Architect", raw, resp, exc) from exc
    if not isinstance(parsed, dict):
        raise PromptForgeError(
            "model",
            "Architect: schema violation — valides JSON, aber kein Objekt "
            f"(Antwort beginnt: {str(parsed)[:160]!r})",
        )
    task = str(parsed.get("task") or "").strip()
    if not task:
        # Kleine Modelle antworten manchmal statt zu extrahieren.
        raise PromptForgeError(
            "model",
            "Architect-IR ohne 'task'-Feld — das Modell hat vermutlich nicht extrahiert "
            f"(Antwort beginnt: {str(parsed)[:120]!r})",
        )
    ir = _normalize_ir(parsed, str(req.get("request_id") or ""))
    return _ok(
        json.dumps(ir, ensure_ascii=False),
        resp,
        system_prompt=ARCHITECT_SYSTEM,
        user_prompt=f"INTENT: {str(intent).strip()}",
    )


def op_optimize(req: dict) -> dict:
    payload = _req_json(req.get("user_prompt") or "")
    long_prompt = payload.get("long_prompt") or ""
    feedback = payload.get("feedback") or []
    ir = payload.get("ir")
    if not isinstance(ir, dict):
        ir = {}
    user_parts = [
        "Optimiere den folgenden Long Prompt. Bewahre alle Constraints und den Output-Contract.",
        "Die Prompt IR ist die kanonische Quelle. Komprimiere, fasse NICHT zusammen.",
        "",
        "LONG PROMPT:",
        "---",
        str(long_prompt),
        "---",
    ]
    # Verpflichtende IR-Inhalte explizit anführen (wörtlich erhalten).
    mandatory = []
    for key, label in (
        ("objective", "OBJECTIVES"),
        ("constraints", "CONSTRAINTS"),
        ("procedure", "PROCEDURE/INSTRUCTIONS"),
        ("verification_requirements", "VERIFICATION REQUIREMENTS"),
        ("context", "CONTEXT"),
        ("assumptions", "ASSUMPTIONS"),
    ):
        values = ir.get(key)
        if isinstance(values, list):
            texts = [str(v).strip() for v in values if str(v).strip()]
            if texts:
                mandatory.append(f"{label}:")
                mandatory.extend(f"- {t}" for t in texts)
    oc = ir.get("output_contract")
    if isinstance(oc, dict):
        oc_lines = []
        if str(oc.get("format") or "").strip():
            oc_lines.append(f"- Format: {str(oc['format']).strip()}")
        for item in oc.get("structure") or []:
            if str(item).strip():
                oc_lines.append(f"- Struktur: {str(item).strip()}")
        for item in oc.get("rules") or []:
            if str(item).strip():
                oc_lines.append(f"- Regel: {str(item).strip()}")
        if oc_lines:
            mandatory.append("OUTPUT CONTRACT:")
            mandatory.extend(oc_lines)
    inputs = ir.get("inputs")
    if isinstance(inputs, list):
        in_lines = []
        for item in inputs:
            if isinstance(item, dict):
                name = str(item.get("name") or "").strip()
                desc = str(item.get("description") or "").strip()
                if name or desc:
                    in_lines.append(f"- {name}: {desc}" if name and desc else f"- {name or desc}")
        if in_lines:
            mandatory.append("EINGABEN (INPUTS):")
            mandatory.extend(in_lines)
    if mandatory:
        user_parts.append("")
        user_parts.append("VERPFLICHTENDE INHALTE — wörtlich erhalten, nicht entfernen, nicht umschreiben:")
        user_parts.extend(mandatory)
    if feedback:
        user_parts.append("")
        user_parts.append("Frühere Verifikation meldete folgende fehlende Inhalte (unbedingt erhalten):")
        user_parts.extend(f"- {item}" for item in feedback)
    user_parts.append("IR-Zusammenfassung:")
    user_parts.append(json.dumps(payload.get("ir", {}), ensure_ascii=False))
    user_text = "\n".join(user_parts)
    provider_kind = (req.get("provider") or "auto").strip().lower()
    if provider_kind == "mock":
        resp = mock_complete("optimize", json.dumps(payload, ensure_ascii=False))
    else:
        messages = [{"role": "system", "content": OPTIMIZE_SYSTEM}, {"role": "user", "content": user_text}]
        kwargs = _common(req)
        kwargs["provider"] = provider_kind
        resp = llm_provider.chat(messages, **kwargs)
    return _ok(
        str(resp.get("content", "")),
        resp,
        system_prompt=OPTIMIZE_SYSTEM,
        user_prompt=user_text,
    )


def op_verify(req: dict) -> dict:
    payload = _req_json(req.get("user_prompt") or "")
    user_text = (
        "Vergleiche den optimierten Prompt mit dem Original-Prompt auf semantische Erhaltung.\n\n"
        "ORIGINAL:\n---\n"
        + str(payload.get("long_prompt", ""))
        + "\n---\n\nOPTIMIERT:\n---\n"
        + str(payload.get("optimized_prompt", ""))
        + "\n---\n\nZu erhaltende Atom-Inhalte:\n"
        + json.dumps(payload.get("atoms", {}), ensure_ascii=False)
    )
    provider_kind = (req.get("provider") or "auto").strip().lower()
    if provider_kind == "mock":
        resp = mock_complete("verify", "{}")
    else:
        messages = [{"role": "system", "content": VERIFY_SYSTEM}, {"role": "user", "content": user_text}]
        kwargs = _common(req)
        kwargs["provider"] = provider_kind
        resp = llm_provider.chat(messages, **kwargs)
    raw = _json_clean(str(resp.get("content", "")))
    try:
        report = json.loads(raw)
    except Exception as exc:
        raise _json_parse_failure("Verify", raw, resp, exc) from exc
    if not isinstance(report, dict):
        raise PromptForgeError("model", "Verify: schema violation — valides JSON, aber kein Objekt")
    # Robustheit gegen unvollständige Reports kleiner Modelle:
    report.setdefault("semantic_preservation", 0.0)
    report.setdefault("constraints_preserved", False)
    report.setdefault("output_contract_preserved", False)
    report.setdefault("objective_preserved", False)
    report.setdefault("instructions_preserved", False)
    report.setdefault("comment", "")
    return _ok(
        json.dumps(report, ensure_ascii=False),
        resp,
        system_prompt=VERIFY_SYSTEM,
        user_prompt=user_text,
    )


def op_chat(req: dict) -> dict:
    payload = _req_json(req.get("user_prompt") or "")
    prompt = payload.get("prompt") or req.get("user_prompt") or ""
    provider_kind = (req.get("provider") or "auto").strip().lower()
    if provider_kind == "mock":
        resp = mock_complete("chat", json.dumps({"prompt": prompt}, ensure_ascii=False))
    else:
        messages = [{"role": "user", "content": str(prompt)}]
        kwargs = _common(req)
        kwargs["provider"] = provider_kind
        resp = llm_provider.chat(messages, **kwargs)
    return _ok(
        str(resp.get("content", "")),
        resp,
        system_prompt=None,
        user_prompt=str(prompt),
    )


# --- Bridge-Einstieg ---

def handle_request(request: str) -> str:
    """Einstiegspunkt Rust→Python (PyO3). Liefert immer eine JSON-Zeichenkette."""
    try:
        req = _req_json(request)
        op = req.get("operation")
        if not isinstance(op, str):
            raise PromptForgeError("bridge", "Request ohne 'operation'")
        request_id = req.get("request_id", "-")
        if op == "architect":
            result = op_architect(req)
        elif op == "optimize":
            result = op_optimize(req)
        elif op == "verify":
            result = op_verify(req)
        elif op == "chat":
            result = op_chat(req)
        else:
            raise PromptForgeError("invalid_input", f"unbekannte Operation: {op}")
        pylog.info("op ok", operation=op, request_id=request_id, duration_ms=result.get("duration_ms"))
        return json.dumps(result, ensure_ascii=False)
    except PromptForgeError as exc:
        pylog.warning("op error", kind=exc.kind, message=exc.message)
        return json.dumps({"ok": False, "error": exc.to_dict()}, ensure_ascii=False)
    except Exception as exc:  # defensiv: nie crashen über die Sprachgrenze
        pylog.error("bridge internal error", error=str(exc))
        return json.dumps(
            {"ok": False, "error": {"kind": "bridge", "message": f"interner Python-Fehler: {exc}"}},
            ensure_ascii=False,
        )
