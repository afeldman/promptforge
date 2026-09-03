"""Tests der Bridge (JSON-Vertrag) mit Mock-Provider — lauffähig ohne any-llm."""

import json
import unittest

from promptforge.bridge import handle_request, _json_clean, _req_json


def make_request(operation, **overrides):
    req = {
        "operation": operation,
        "user_prompt": "",
        "provider": "mock",
        "model": "mock-model",
        "endpoint": None,
        "api_key": None,
        "temperature": None,
        "max_tokens": None,
        "timeout_s": 30,
        "request_id": "test-rid",
    }
    req.update(overrides)
    return json.dumps(req)


class BridgeArchitectTests(unittest.TestCase):
    def test_architect_returns_ok_ir(self):
        payload = json.dumps({"intent": "Analysiere fünf Papers und vergleiche die Methoden"}, ensure_ascii=False)
        out = json.loads(handle_request(make_request("architect", user_prompt=payload)))
        self.assertTrue(out["ok"])
        ir = json.loads(out["content"])
        self.assertEqual(ir["schema_version"], 1)
        self.assertIn("Papers", ir["task"])
        self.assertTrue(ir["objective"])
        self.assertEqual(out["usage"]["prompt_tokens"], 10)
        self.assertEqual(out["model"], "mock")

    def test_architect_empty_intent_error(self):
        out = json.loads(handle_request(make_request("architect", user_prompt="{}")))
        self.assertFalse(out["ok"])
        self.assertEqual(out["error"]["kind"], "invalid_input")


class BridgeOptimizeVerifyTests(unittest.TestCase):
    def test_optimize_returns_prompt_json(self):
        long_prompt = "## Aufgabe\nVergleiche fünf Papers\n## Ziele\nMethoden vergleichen\n"
        payload = json.dumps({"ir": {"task": "x"}, "long_prompt": long_prompt, "feedback": []})
        out = json.loads(handle_request(make_request("optimize", user_prompt=payload)))
        self.assertTrue(out["ok"])
        parsed = json.loads(out["content"])
        self.assertIn("Vergleiche fünf Papers", parsed["prompt"])

    def test_verify_returns_semantic_report(self):
        payload = json.dumps(
            {"atoms": {"constraints": ["Nur peer-reviewte Quellen"]}, "long_prompt": "orig", "optimized_prompt": "opt"}
        )
        out = json.loads(handle_request(make_request("verify", user_prompt=payload)))
        self.assertTrue(out["ok"])
        report = json.loads(out["content"])
        self.assertAlmostEqual(report["semantic_preservation"], 0.98)
        self.assertTrue(report["constraints_preserved"])


class BridgeChatTests(unittest.TestCase):
    def test_chat_returns_mock_answer(self):
        payload = json.dumps({"prompt": "Fasse zusammen"})
        out = json.loads(handle_request(make_request("chat", user_prompt=payload)))
        self.assertTrue(out["ok"])
        self.assertTrue(out["content"].startswith("Mock-Antwort"))


class BridgeErrorHandlingTests(unittest.TestCase):
    def test_unknown_operation(self):
        out = json.loads(handle_request(make_request("teleport")))
        self.assertFalse(out["ok"])
        self.assertEqual(out["error"]["kind"], "invalid_input")

    def test_non_json_request(self):
        out = json.loads(handle_request("kein json"))
        self.assertFalse(out["ok"])
        self.assertEqual(out["error"]["kind"], "bridge")


class BridgeHelpersTests(unittest.TestCase):
    def test_json_clean_strips_fences(self):
        self.assertEqual(_json_clean('```json\n{"a": 1}\n```'), '{"a": 1}')
        self.assertEqual(_json_clean('{"a": 1}'), '{"a": 1}')

    def test_req_json_validation(self):
        with self.assertRaises(Exception):
            _req_json("nope")
        self.assertEqual(_req_json('{"a": 1}')["a"], 1)


# --- Repair CI/apfel: Architect-Response-Contract (Truncation-Diagnostik) ---
# Echte Python-Bridge-Pfade (op_architect/op_verify), nur der Provider-Chat
# wird gepatched — Rust→Python→Provider→Python→Rust bleibt echt getestet.

import promptforge.bridge as bridge
from unittest import mock


def _chat_result(content, finish_reason=None):
    return {"content": content, "finish_reason": finish_reason, "model": "test", "usage": None}


def _architect_request_with_chat(content, finish_reason=None):
    payload = json.dumps({"intent": "auditiere das projekt"}, ensure_ascii=False)
    with mock.patch.object(bridge.llm_provider, "chat", return_value=_chat_result(content, finish_reason)) as m:
        out = json.loads(handle_request(make_request("architect", provider="auto", user_prompt=payload)))
    return out, m


class ArchitectResponseContractTests(unittest.TestCase):
    def test_valid_json_becomes_ir(self):
        content = json.dumps(
            {"schema_version": 1, "task": "Auditiere das Projekt", "objective": ["Risiken finden"],
             "constraints": [], "procedure": ["Struktur prüfen"], "output_contract": {"format": "markdown"},
             "verification_requirements": [], "metadata": {}}, ensure_ascii=False
        )
        out, _ = _architect_request_with_chat(content)
        self.assertTrue(out["ok"], out)
        ir = json.loads(out["content"])
        self.assertEqual(ir["task"], "Auditiere das Projekt")

    def test_truncated_json_diagnosed(self):
        content = '{"schema_version":"1","task":"audit","objective":["a","b"],"procedure":["x",'
        out, _ = _architect_request_with_chat(content)
        self.assertFalse(out["ok"])
        self.assertEqual(out["error"]["kind"], "model")
        self.assertIn("truncated", out["error"]["message"].lower())

    def test_truncated_json_with_finish_reason_length(self):
        content = '{"schema_version":"1","task":"audit","objective":[' * 1
        out, _ = _architect_request_with_chat(content, finish_reason="length")
        self.assertFalse(out["ok"])
        self.assertIn("finish_reason=length", out["error"]["message"])

    def test_empty_response_diagnosed(self):
        out, _ = _architect_request_with_chat("")
        self.assertFalse(out["ok"])
        self.assertIn("empty response", out["error"]["message"])

    def test_schema_violation_diagnosed(self):
        out, _ = _architect_request_with_chat('["kein", "objekt"]')
        self.assertFalse(out["ok"])
        self.assertIn("schema violation", out["error"]["message"])

    def test_verify_truncated_diagnosed(self):
        payload = json.dumps({"atoms": {"constraints": ["c"]}, "long_prompt": "orig", "optimized_prompt": "opt"})
        with mock.patch.object(bridge.llm_provider, "chat",
                               return_value=_chat_result('{"semantic_preservation": 0.9, "constraints_preserved": tru')):
            out = json.loads(handle_request(make_request("verify", provider="auto", user_prompt=payload)))
        self.assertFalse(out["ok"])
        self.assertIn("truncated", out["error"]["message"].lower())

    def test_system_and_user_prompt_echoed_on_success(self):
        content = json.dumps(
            {"schema_version": 1, "task": "Auditiere", "objective": [], "constraints": [],
             "procedure": [], "output_contract": {"format": "markdown"}, "verification_requirements": [],
             "metadata": {}}, ensure_ascii=False
        )
        out, m = _architect_request_with_chat(content)
        self.assertTrue(out["ok"])
        # Echter Request: System- + User-Prompt wurden tatsächlich gesendet.
        sent = m.call_args
        self.assertIsNotNone(sent)
        msgs = sent.args[0]
        self.assertEqual(msgs[0]["role"], "system")
        self.assertIn("Prompt-Architect", msgs[0]["content"])
        self.assertIn("auditiere das projekt", msgs[1]["content"])
        # Echo im Antwort-JSON (Debug-Trace-Pfad).
        self.assertIn("system_prompt", out)
        self.assertIn("user_prompt", out)


if __name__ == "__main__":
    unittest.main()
