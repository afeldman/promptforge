"""Tests des Mock-LLM (deterministisch) und der any-llm-Kapselung.

any-llm-Tests laufen nur, wenn `any_llm` installiert ist; ohne Installation
wird geprüft, dass ein klarer ConfigurationError entsteht."""

import json
import unittest

from promptforge.errors import PromptForgeError
from promptforge.llm.mock import mock_complete, _basic_ir
from promptforge.llm import provider


class MockLlmTests(unittest.TestCase):
    def test_architect_ir_shape(self):
        resp = mock_complete("architect", json.dumps({"intent": "Vergleiche Methoden"}))
        ir = json.loads(resp["content"])
        self.assertEqual(ir["schema_version"], 1)
        self.assertIn("Vergleiche Methoden", ir["task"])
        self.assertEqual(ir["output_contract"]["format"], "markdown")

    def test_optimize_normalizes_whitespace(self):
        long_text = "Zeile1\n\n\n\nZeile2\n"
        resp = mock_complete("optimize", json.dumps({"long_prompt": long_text}))
        parsed = json.loads(resp["content"])
        self.assertEqual(parsed["prompt"], "Zeile1\n\nZeile2\n")

    def test_basic_ir_deterministic(self):
        self.assertEqual(_basic_ir("A"), _basic_ir("A"))


class ProviderDispatchTests(unittest.TestCase):
    def test_unknown_provider_raises_config(self):
        with self.assertRaises(PromptForgeError) as ctx:
            provider.chat([], provider="klingonisch", model="x")
        self.assertEqual(ctx.exception.kind, "configuration")

    def test_mock_chat_dispatch(self):
        out = provider.chat(
            [{"role": "user", "content": "Hallo"}], provider="mock"
        )
        self.assertIn("Mock-Antwort", out["content"])

    def test_missing_model_with_anyllm_raises_config(self):
        with self.assertRaises(PromptForgeError) as ctx:
            provider.chat([], provider="any_llm", endpoint=None, model=None)
        self.assertEqual(ctx.exception.kind, "configuration")

    def test_anyllm_without_install_raises_config(self):
        try:
            import any_llm  # noqa: F401
        except ImportError:
            with self.assertRaises(PromptForgeError) as ctx:
                provider.chat([{"role": "user", "content": "hi"}], provider="auto",
                              endpoint="http://localhost:11434/v1", model="m")
            self.assertEqual(ctx.exception.kind, "configuration")
            return
        self.skipTest("any-llm installiert — Fallback-Test übersprungen")


if __name__ == "__main__":
    unittest.main()
