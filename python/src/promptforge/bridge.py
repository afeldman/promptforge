"""Operationen (Architect/Optimize/Verify) + Bridge-Einstieg (Rust→Python).

`handle_request(request: str) -> str` ist der JSON-Vertrag über PyO3."""

import json
import os
import time

from . import logging as pylog
from .errors import PromptForgeError
from .llm import provider as llm_provider
from .llm.mock import mock_complete
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


def _ok(content: str, resp: dict) -> dict:
    out = {"ok": True, "content": content}
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
        messages = [{"role": "system", "content": ARCHITECT_SYSTEM}, {"role": "user", "content": str(intent)}]
        kwargs = _common(req)
        kwargs["provider"] = provider_kind
        resp = llm_provider.chat(messages, **kwargs)
    raw = _json_clean(str(resp.get("content", "")))
    try:
        ir = json.loads(raw)
    except Exception as exc:
        raise PromptForgeError("model", f"Architect lieferte kein JSON: {exc}") from exc
    if not isinstance(ir, dict) or not ir.get("task"):
        raise PromptForgeError("model", "Architect-IR ohne 'task'-Feld")
    return _ok(json.dumps(ir, ensure_ascii=False), resp)


def op_optimize(req: dict) -> dict:
    payload = _req_json(req.get("user_prompt") or "")
    long_prompt = payload.get("long_prompt") or ""
    feedback = payload.get("feedback") or []
    user_parts = [
        "Optimiere den folgenden Long Prompt. Bewahre alle Constraints und den Output-Contract.",
        "",
        "LONG PROMPT:",
        "---",
        str(long_prompt),
        "---",
    ]
    if feedback:
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
    return _ok(str(resp.get("content", "")), resp)


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
    return _ok(str(resp.get("content", "")), resp)


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
    return _ok(str(resp.get("content", "")), resp)


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
