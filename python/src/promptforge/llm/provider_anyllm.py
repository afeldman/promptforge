"""any-llm-Kapselung (Spec §2).

any-llm dringt nicht in den Rust-Core: Diese Datei ist die einzige Stelle,
die `any_llm` importiert (lazy). Konfiguration kommt aus LLM_ENDPOINT /
LLM_KEY / LLM_MODEL bzw. aus dem Bridge-Request."""

import os
from typing import Any

from ..errors import PromptForgeError

# Import bewusst lazy: das Paket ist optional (Mock/Tests ohne any-llm).


def _anyllm_module():
    # any-llm: einheitliche Exception-Taxonomie aktivieren (Zukunftssicherheit).
    os.environ.setdefault("ANY_LLM_UNIFIED_EXCEPTIONS", "1")
    try:
        import any_llm  # type: ignore
    except Exception as exc:  # pragma: no cover - nur ohne Installation
        raise PromptForgeError(
            "configuration",
            "any-llm-sdk ist nicht installiert. Bitte `uv sync` im python/-Verzeichnis ausführen "
            "(siehe README → Development Setup).",
        ) from exc
    return any_llm


def _build_client(endpoint: str | None, api_key: str | None, model: str | None):
    any_llm = _anyllm_module()
    if not model:
        raise PromptForgeError("configuration", "LLM_MODEL ist nicht gesetzt")
    try:
        if endpoint:
            # Bevorzugter Weg für beliebige OpenAI-kompatible Endpunkte
            # (lokal: Ollama/LM Studio/llama.cpp; Cloud-Gateways).
            create = getattr(any_llm.AnyLLM, "create_openai_compatible", None)
            if create is not None:
                return create(name="custom", api_base=endpoint, api_key=api_key)
            # Fallback: openai-Provider mit api_base.
            return any_llm.AnyLLM.create("openai", api_key=api_key, api_base=endpoint)
        # Ohne Endpunkt: Provider-Name aus Modellnamen ("provider:model").
        provider = model.split(":", 1)[0] if ":" in model else "openai"
        return any_llm.AnyLLM.create(provider, api_key=api_key)
    except PromptForgeError:
        raise
    except Exception as exc:
        raise PromptForgeError("provider", f"Client-Erstellung fehlgeschlagen: {exc}") from exc


def _normalize_response(resp: Any) -> dict:
    """Normalisiert any-llm-Antworten auf {content, usage, finish_reason, model}."""
    # OpenAI-kompatibles Objekt: choices[0].message.content
    choices = getattr(resp, "choices", None)
    content = None
    if choices:
        msg = getattr(choices[0], "message", None)
        content = getattr(msg, "content", None) if msg is not None else None
    if content is None:
        content = getattr(resp, "content", None)
    if content is None:
        raise PromptForgeError("provider", "Antwort ohne Inhalt (content)")

    usage = getattr(resp, "usage", None)
    usage_out = None
    if usage is not None:
        usage_out = {
            "prompt_tokens": getattr(usage, "prompt_tokens", None),
            "completion_tokens": getattr(usage, "completion_tokens", None),
        }

    return {
        "content": content,
        "usage": usage_out,
        "finish_reason": getattr(resp, "finish_reason", None),
        "model": getattr(resp, "model", None),
    }


def _map_anyllm_error(exc: Exception) -> PromptForgeError:
    try:
        import any_llm.exceptions as ae  # type: ignore
    except Exception:
        ae = None

    if ae is not None:
        mapping = [
            (getattr(ae, "AuthenticationError", ()), "authentication"),
            (getattr(ae, "RateLimitError", ()), "provider"),
            (getattr(ae, "ModelNotFoundError", ()), "model"),
            (getattr(ae, "ContextLengthExceededError", ()), "model"),
            (getattr(ae, "InvalidRequestError", ()), "provider"),
            (getattr(ae, "ProviderError", ()), "provider"),
            (getattr(ae, "AnyLLMError", ()), "provider"),
        ]
        for klass, kind in mapping:
            if klass and isinstance(exc, klass):
                return PromptForgeError(kind, str(exc))
    return PromptForgeError("provider", str(exc))


def anyllm_chat(
    messages: list[dict],
    *,
    endpoint: str | None,
    api_key: str | None,
    model: str,
    temperature: float | None,
    max_tokens: int | None,
    timeout_s: int,
) -> dict:
    client = _build_client(endpoint, api_key, model)
    kwargs: dict[str, Any] = {"model": model, "messages": messages}
    if temperature is not None:
        kwargs["temperature"] = temperature
    if max_tokens is not None:
        kwargs["max_tokens"] = max_tokens
    if timeout_s and timeout_s > 0:
        kwargs["timeout"] = timeout_s
    try:
        resp = client.completion(**kwargs)
        return _normalize_response(resp)
    except Exception as exc:
        raise _map_anyllm_error(exc) from exc
