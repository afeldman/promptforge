"""LLM-Schicht von PromptForge (Spec §2/§3).

Kapselt Provider (any-llm für OpenAI-kompatible Endpunkte, Mock für
Tests/Demo). Rust kennt nur den JSON-Vertrag über `promptforge.bridge`.
"""

from .provider import chat  # noqa: F401
