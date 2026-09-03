"""Provider-Dispatch: mock | any_llm | auto.

`chat()` liefert ein normalisiertes Ergebnis-Dict:
{content, usage?, finish_reason?, model?} oder wirft PromptForgeError."""

from ..errors import PromptForgeError
from . import provider_anyllm
from .mock import mock_complete


def chat(
    messages: list[dict],
    *,
    provider: str = "auto",
    endpoint: str | None = None,
    api_key: str | None = None,
    model: str | None = None,
    temperature: float | None = None,
    max_tokens: int | None = None,
    timeout_s: int = 120,
) -> dict:
    kind = (provider or "auto").strip().lower()
    if kind == "mock":
        # Für den Mock ist nur der letzte User-Inhalt relevant.
        user_prompt = "".join(m.get("content", "") for m in messages if m.get("role") == "user")
        return mock_complete("chat", user_prompt)

    if kind in ("auto", "any_llm", "anyllm", "any-llm"):
        if not model:
            raise PromptForgeError("configuration", "LLM_MODEL ist nicht gesetzt")
        return provider_anyllm.anyllm_chat(
            messages,
            endpoint=endpoint,
            api_key=api_key,
            model=model,
            temperature=temperature,
            max_tokens=max_tokens,
            timeout_s=timeout_s,
        )

    raise PromptForgeError("configuration", f"unbekannter Provider: {provider}")
