"""Minimales Logging: loguru, wenn installiert; sonst stdlib (kein Import-Fehler).

Beide Seiten (Rust tracing / Python) loggen strukturierte Metadaten
(request_id, stage, model, duration, token counts) — niemals Secrets oder
komplette Prompts (opt-in über PF_PROMPT_LOG)."""

import logging
import os
import sys
from typing import Any, cast

try:  # pragma: no cover - hängt von Installation ab
    from loguru import logger as _loguru  # type: ignore

    _HAS_LOGURU = True
except Exception:  # pragma: no cover
    _loguru = None
    _HAS_LOGURU = False

# Loguru ist optional; cast beruhigt die statische Analyse.
_LG: Any = cast(Any, _loguru)


def _configured() -> bool:
    return bool(os.environ.get("_PF_PY_LOGGING_READY"))


def configure(level: str = "INFO", fmt: str = "text", log_dir: str | None = None, to_stderr: bool = False) -> None:
    """Konfiguriert die Python-Logseite (einmalig, idempotent).

    Ohne `log_dir`/`to_stderr` bleibt die Python-Logseite still (kein
    Stderr-Rauschen im eingebetteten Rust-Kontext); die Rust-Seite loggt
    ohnehin strukturiert in Rolling-Dateien.
    """
    if _configured():
        return
    os.environ["_PF_PY_LOGGING_READY"] = "1"
    if not _HAS_LOGURU:
        return
    level = level.upper()
    _LG.remove()
    fmt_json = fmt.lower() == "json"
    if to_stderr:
        _LG.add(sys.stderr, level=level, serialize=fmt_json)
    if log_dir:
        import pathlib

        pathlib.Path(log_dir).mkdir(parents=True, exist_ok=True)
        kwargs: dict[str, Any] = {"level": level, "rotation": "10 MB"}
        if fmt_json:
            kwargs["serialize"] = True
        else:
            kwargs["format"] = "<green>{time:HH:mm:ss.SSS}</green> | <level>{level: <8}</level> | {message}"
        _LG.add(str(pathlib.Path(log_dir) / "promptforge-py.log"), **kwargs)


def info(msg: str, **fields) -> None:
    extra = _fmt(fields)
    if _HAS_LOGURU:
        _LG.info(f"{msg}{extra}")
    else:
        logging.getLogger("promptforge").info("%s%s", msg, extra)


def warning(msg: str, **fields) -> None:
    extra = _fmt(fields)
    if _HAS_LOGURU:
        _LG.warning(f"{msg}{extra}")
    else:
        logging.getLogger("promptforge").warning("%s%s", msg, extra)


def error(msg: str, **fields) -> None:
    extra = _fmt(fields)
    if _HAS_LOGURU:
        _LG.error(f"{msg}{extra}")
    else:
        logging.getLogger("promptforge").error("%s%s", msg, extra)


def _fmt(fields: dict) -> str:
    if not fields:
        return ""
    parts = [f"{k}={v}" for k, v in sorted(fields.items()) if v is not None]
    return " | " + " ".join(parts) if parts else ""
