"""Tests Fehler-/Provider-Schicht (kein Netz, kein any-llm nötig)."""

import unittest

from promptforge.errors import PromptForgeError, KNOWN_KINDS


class ErrorTests(unittest.TestCase):
    def test_known_kinds_cover_rust_mapping(self):
        for kind in ["configuration", "provider", "authentication", "model", "timeout",
                     "tokenization", "optimization", "verification", "persistence",
                     "bridge", "invalid_input"]:
            self.assertIn(kind, KNOWN_KINDS)

    def test_error_dict_shape(self):
        e = PromptForgeError("timeout", "zu langsam")
        self.assertEqual(e.to_dict(), {"kind": "timeout", "message": "zu langsam"})

    def test_unknown_kind_defaults_to_provider(self):
        e = PromptForgeError("weird", "x")
        self.assertEqual(e.kind, "provider")


if __name__ == "__main__":
    unittest.main()
