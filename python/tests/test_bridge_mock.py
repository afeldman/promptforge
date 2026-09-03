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


if __name__ == "__main__":
    unittest.main()
