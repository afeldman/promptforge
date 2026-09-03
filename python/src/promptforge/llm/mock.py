"""Deterministischer Mock-LLM (kein Netz; Tests & Demo).

Hält denselben JSON-Vertrag ein wie die any-llm-Provider-Schicht. Der
Mock spiegelt die Semantik des Rust-Mock (pf_engine::mock), damit Tests
deterministisch sind."""

import json

from ..errors import PromptForgeError

IR_SCHEMA = {
    "schema_version": 1,
    "task": "",
    "objective": [],
    "context": [],
    "inputs": [],
    "constraints": [],
    "assumptions": [],
    "role": None,
    "procedure": [],
    "reasoning_strategy": None,
    "examples": [],
    "output_contract": {"format": "markdown", "structure": [], "rules": [], "example": None},
    "verification_requirements": [],
    "target_model": None,
    "metadata": {"request_id": "mock", "created_at": "mock", "source_language": None, "tags": [], "engine_version": "mock"},
}


def _basic_ir(intent: str) -> dict:
    ir = json.loads(json.dumps(IR_SCHEMA))
    ir["task"] = intent
    ir["objective"] = [f"Erfülle die folgende Aufgabe vollständig und korrekt: {intent}"]
    ir["role"] = "Wissenschaftlicher Berater (Mock)"
    ir["reasoning_strategy"] = "Schritt-für-Schritt-Analyse"
    ir["output_contract"]["structure"] = ["Ergebnis", "Begründung (falls sinnvoll)"]
    ir["output_contract"]["rules"] = ["Antworte präzise und direkt auf die Aufgabe", "Keine irrelevante Einleitung"]
    return ir


def _normalize_whitespace(text: str) -> str:
    out_lines = []
    blank_run = 0
    for raw in text.splitlines():
        line = raw.rstrip() if raw.startswith(("    ", "\t")) else raw.strip()
        if not line.strip():
            blank_run += 1
            continue
        if out_lines and blank_run > 0:
            out_lines.append("")
        blank_run = 0
        out_lines.append(line)
    return "\n".join(out_lines) + "\n"


def mock_complete(operation: str, user_prompt: str) -> dict:
    """Liefert {content, usage, model, finish_reason} deterministisch."""
    if operation == "architect":
        try:
            payload = json.loads(user_prompt)
            intent = payload.get("intent") or user_prompt
        except Exception:
            intent = user_prompt
        content = json.dumps(_basic_ir(str(intent)), ensure_ascii=False)
    elif operation == "optimize":
        try:
            payload = json.loads(user_prompt)
            long_prompt = payload.get("long_prompt") or user_prompt
        except Exception:
            long_prompt = user_prompt
        # Mock-Optimierung: nur Whitespace-Normalisierung (Inhalt bleibt).
        prompt = _normalize_whitespace(str(long_prompt))
        content = json.dumps({"prompt": prompt, "notes": ["mock: whitespace"]}, ensure_ascii=False)
    elif operation == "verify":
        content = json.dumps(
            {
                "semantic_preservation": 0.98,
                "constraints_preserved": True,
                "output_contract_preserved": True,
                "objective_preserved": True,
                "instructions_preserved": True,
                "comment": "mock: semantisch erhalten",
            },
            ensure_ascii=False,
        )
    elif operation == "chat":
        try:
            payload = json.loads(user_prompt)
            text = payload.get("prompt") or user_prompt
        except Exception:
            text = user_prompt
        content = f"Mock-Antwort auf: {str(text)[:80]}"
    else:  # pragma: no cover - durch Bridge-Dispatch verhindert
        raise PromptForgeError("invalid_input", f"unbekannte Operation: {operation}")

    return {
        "content": content,
        "usage": {"prompt_tokens": 10, "completion_tokens": len(content.split())},
        "model": "mock",
        "finish_reason": "stop",
    }
