"""PromptForge — Python AI layer.

Diese Schicht kapselt alle LLM-Kontakte (Provider, Mock, any-llm) und die
LLM-gestützten Pipeline-Passes (Architect, Optimize, Verify). Sie wird vom
Rust-Core über PyO3 mit JSON-Requests aufgerufen (siehe `promptforge.bridge`).
"""

__version__ = "0.1.0"
