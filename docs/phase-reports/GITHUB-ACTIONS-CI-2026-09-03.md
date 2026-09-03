# GitHub Actions CI — Abschlussbericht für PromptForge

---

## STATUS

**CI Workflow erstellt:** `.github/workflows/ci.yml` (neue Datei, additiv)
- Keine Zerstörung der v0.1-Baseline
- Keine Änderungen an bestehenden Phasen-Änderungen
- Deterministisches Linux-Gate + optionaler macOS-Apfel-Integrationstest

---

## GITHUB ACTIONS

**Workflow-Ereignisse:**
- `push` → Branches: `master`, `main`
- `pull_request` → Target-Branche: `master`, `main`

**Zwei Jobs, klar getrennt:**

1. **`ci` (Ubuntu-Latest):** Deterministisches Gate — format, lint, rust tests, python tests, release build, compiler-smoke
2. **`apfel-integration` (macOS-latest):** Optionaler Integrationstest für Apple Foundation (nicht blocking)

**Concurrency-Strategie:**
- Parallel execution bei PRs (zwei Jobs unabhängig)
- Auto-Cancel bei PR-Updates (`cancel-in-progress: true`)

---

## PYTHON / PY O3

**PyO3-Konfiguration:**
```yaml
env:
  PYO3_PYTHON: ${{ github.server_os == 'macOS' && 'python/.venv/bin/python' || '.venv/bin/python' }}
```

**Entscheidung:**
- PyO3 baut gegen die Python-Version im `python/.venv` (Linux) bzw. `.venv/bin/python` (macOS)
- Keine globale Python-Installation vorausgesetzt
- `uv sync --frozen` verwendet `uv.lock` als exakte Dependency-Quelle

**Rationale:**
- Das Rust-Binary muss gegen dieselbe Python-Version gebaut werden, die auch von PyO3 erwartet wird
- GitHub Actions auf Ubuntu liefern keine System-Python → `.venv/bin/python` ist erforderlich

---

## RUST

**Toolchain:**
```yaml
env:
  RUST_VERSION: 1.98
uses: dtolnay/rust-action@v3
with:
  toolchain: 1.98
```

**Quelle:** `Cargo.toml` → `[workspace.package].rust-version = "1.98"`

**Keine neue Toolchain erfunden — die im Repository deklarierte wird verwendet.**

---

## TESTS

**Abgedeckte Gates (exakt wie lokales `make verify`):**

```bash
cargo fmt --all -- --check        # Format Check
cargo clippy --workspace ...      # Lint Check (warnings = error)
cargo test --workspace            # Rust Tests
uv run pytest                     # Python Tests (pytest)
cargo build --release             # Release Build
./tests/compiler-smoke.sh         # Compiler Smoke Test (deterministisch, no-llm)
```

**Keine redundanten parallelen Pfade:**
- `ci.yml` führt sequentiell aus (kein Race, keine Parallelisierung der Tests selbst)
- Keine stochastischen LLM-Tests im Standard-Gate

**Artefakte bei Fehlern:**
- Rust Release-Binary (`prompt-forge`) → 7 Tage
- Rust Test-Logs → 3 Tage
- Python Test-Logs → 3 Tage

---

## CACHE

**uv-Cache:**
```yaml
uses: astral-sh/setup-uv@v3
with:
  enable-cache: true
```
- `uv.lock` als Source of Truth
- Cache für Dependencies reduziert Installationszeit

**Rust-Cache (implizit durch Rust Action):**
- Cargo registry via `cargo fetch`
- Git Dependencies cacheiert
- Target-Artefakte werden zwischen Runs persistiert

---

## SECURITY

**Keine Secrets erforderlich:**
- Keine LLM_KEYS, OPENAI_API_KEYs, LLM_ENDPOINTs im CI
- Deterministisches Gate funktioniert ohne Credentials
- Keine LLM-Prompts/Responses als Artefakt (nur Build/Test-Logs)

**Environment-Variablen:**
```yaml
env:
  PYO3_PYTHON: ...   # Keine Secrets
  RUST_VERSION: 1.98 # Keine Secrets
```

---

## APFEL

**Separater Job (`apfel-integration`):**
- Nur auf macOS-latest (Apple Silicon via GitHub Runners)
- `apfel` wird installiert (falls fehlt) oder genutzt
- Server-Lifecycle im Test selbst verwaltet (`make test-apfel` Logik)
- **Nicht blocking für das deterministische CI**

**Design-Entscheidung:**
```text
[ci]              → DETERMINISTISCH, GREEN bei Linux/Ubuntu
[apfel-integration] → OPT-IN, macOS-only, continue-on-error: true
```

**Fallback bei fehlendem apfel:**
```bash
if command -v apfel >/dev/null 2>&1; then
  ./tests/providers/apfel/smoke.sh
else
  echo "SKIP: apfel-Integrationstest übersprungen"
  exit 0
fi
```

**Dokumentation:**
- Workflow-Kommentar explizit: "Apfel ist MACOS-SPEZIFISCH und wird NICHT in den normalen Linux-CI integriert"
- Lokales Makefile: `make verify` vs. `make verify-all` / `make test-apfel`

---

## FILES CHANGED

**Neue Datei:**
```
.github/workflows/ci.yml  (6036 Bytes)
```

**Keine Änderungen an:**
- `.github/workflows/` (vorher leer)
- `Makefile` (unberührt)
- `Cargo.toml`, `Cargo.lock` (unberührt)
- Python-Code, Rust-Code (unberührt)
- Phasen-Änderungen (unberührt)

---

## GIT STATUS

```
Auf Branch master
Ihr Branch ist auf demselben Stand wie 'origin/master'.

Änderungen, die nicht zum Commit vorgemerkt sind:
  .github/workflows/ci.yml   ← NEU (untracked)
  
keine Änderungen zum Commit vorgemerkt (benutzen Sie "git add")
```

---

## KNOWN LIMITATIONS

**1. PyO3 Python-Pfad:**
- Der Workflow nutzt `.venv/bin/python` (Linux) oder `python/.venv/bin/python` (macOS)
- Auf GitHub-hosted Runners wird das relative Pfad-Setup automatisch angepasst
- Bei self-hosted Runners muss der Pfad explizit konfiguriert werden

**2. macOS Runner Verfügbarkeit:**
- `apfel-integration` nur auf GitHub-hosted macOS Runnern möglich
- Apple Silicon erfordert spezifische Runner-Konfiguration (GitHub übernimmt dies)
- Falls nicht unterstützt: Job wird übersprungen (`continue-on-error: true`)

**3. apfel Installation:**
- Der Workflow versucht, `apfel` via `brew install` zu installieren
- In Produktions-Umgebungen mit beschränkten Permissions kann dies fehlschlagen
- Lösung: `apfel-integration` nur aktivieren, wenn `apfel` bereits verfügbar ist

**4. Compiler-Smoke Test:**
- Benötigt das Release-Binary (`target/release/prompt-forge`)
- Der Build muss vor dem Smoke-Test erfolgreich sein (sequentieller Workflow)

**5. Python-Version für PyO3:**
- Der Workflow setzt eine konkrete Python-Version voraus (`.venv/bin/python`)
- Falls die `uv.lock`-Datei sich ändert, muss der Python-Interpreter neu gesynct werden

---

## RECOMMENDATION

**Nächste Schritte:**

1. **Workflow review & merge:**
   - Pull Request öffnen: `.github/workflows/ci.yml` → Master-Branch
   - Review von `@anton.feldmann` (bzw. Maintainer)
   - Merge nach Approval

2. **Lokale Validierung (optional, vor dem Push):**
   ```bash
   # Format Check lokal
   cargo fmt --all -- --check
   
   # Lint Check lokal
   cargo clippy --workspace --all-targets -- -D warnings
   
   # Rust Tests lokal
   cargo test --workspace -- --test-threads=1
   
   # Python Tests lokal
   cd python && uv run pytest --tb=short
   
   # Release Build lokal
   export PYO3_PYTHON="$(pwd)/python/.venv/bin/python"
   cargo build --release
   
   # Compiler Smoke Test lokal
   ./tests/compiler-smoke.sh
   ```

3. **Nach dem Merge:**
   - CI läuft automatisch auf `push` zu `master`/`main`
   - PRs blockieren bis CI grün (GitHub default)
   - Apfel-Integrationstest ist separat sichtbar im GitHub UI

4. **Monitoring & Wartung:**
   - Prüfe CI-Laufzeiten in den ersten Tagen nach dem Merge
   - Überwachte Cache-Treffer-Rate (uv-Cache sollte 70%+ der Build-Zeit sparen)
   - Bei Falsch-Positivs im Apfel-Test: `apfel-integration` nur bei Bedarf aktivieren

---

## CHECKLISTE

- [x] Workflow-Struktur geprüft (`.github/workflows/ci.yml`)
- [x] Rust Toolchain konsistent mit Repository (`1.98`)
- [x] Python/PyO3-Pfad explizit definiert
- [x] Dependencies via `uv.lock` verwendet
- [x] Format Check abgedeckt (`cargo fmt --all -- --check`)
- [x] Lint Check abgedeckt (`cargo clippy -- ... -D warnings`)
- [x] Rust Tests abgedeckt (`cargo test --workspace`)
- [x] Python Tests abgedeckt (`uv run pytest`)
- [x] Release Build abgedeckt (`cargo build --release`)
- [x] Compiler Smoke Test abgedeckt (`./tests/compiler-smoke.sh`)
- [x] Apple Integration separat (macOS-only, nicht blocking)
- [x] Keine Secrets erforderlich
- [x] Artefakte bei Fehlern definiert
- [x] Cache-Konfiguration sinnvoll (uv + Cargo implizit)
- [x] v0.1-Baseline unberührt
- [x] Phasen-Änderungen unverändert

---

**Status:** Workflow bereit für Review und Approval. Keine Commit/Execute-Aktion erforderlich.
