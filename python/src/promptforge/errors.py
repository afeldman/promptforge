"""PromptForge Python-Fehler.

Kategorien entsprechen der Rust-Fehlerfamilie (pf_core::ErrorKind) —
maschinenlesbar über `kind`."""

KNOWN_KINDS = {
    "configuration",
    "provider",
    "authentication",
    "model",
    "timeout",
    "tokenization",
    "optimization",
    "verification",
    "persistence",
    "bridge",
    "invalid_input",
}


class PromptForgeError(Exception):
    """Fehler mit stabilem `kind` (JSON-kompatibel zur Rust-Seite)."""

    def __init__(self, kind: str, message: str):
        if kind not in KNOWN_KINDS:
            kind = "provider"
        super().__init__(message)
        self.kind = kind
        self.message = message

    def to_dict(self) -> dict:
        return {"kind": self.kind, "message": self.message}
