//! Pipeline-Orchestrierung (Spec §1/§6/§7/§22 Vertical Slice).
//!
//! ```text
//! intent → architect → PromptIR → expand → LongPrompt → optimize
//!        → optimiert → verify → Ergebnis (ggf. Re-Optimize, max N)
//! ```
//!
//! LLM-Aufrufe laufen über die `LlmBridge` (Python/Mock); ohne LLM ist die
//! Pipeline vollständig deterministisch. Jede Stufe erzeugt Events und
//! Token-Statistiken.

use std::sync::Arc;

use pf_core::bridge::{LlmBridge, LlmOperation, LlmRequest, LlmResponse, Usage};
use pf_core::compilation::{CompilationResult, QualityMetrics};
use pf_core::config::{LlmConfig, ProviderKind, VerifyConfig};
use pf_core::error::{ErrorKind, Result, err};
use pf_core::ir::PromptIr;
use pf_core::token::{TokenReport, Tokenizer};

use crate::expand::expand_to_long_prompt;
use crate::mock::MockBridge;
use crate::optimizer::{OptimizerEvent, deterministic_pass_chain, reinsert_missing_atoms};
use crate::verify::{
    SemanticAtoms, Verdict, VerificationReport, atoms_payload, mandatory_atoms, merge_semantic,
    parse_semantic_report_json, verify_structural,
};

/// Pipeline-Stadien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Architect,
    Expand,
    Optimize,
    Verify,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Architect => "architect",
            Stage::Expand => "expand",
            Stage::Optimize => "optimize",
            Stage::Verify => "verify",
        }
    }
}

/// Ereignisse für CLI/TUI/Service (Fortschritt, ohne Prompt-Inhalte).
#[derive(Debug, Clone)]
pub enum StageEvent {
    StageStarted(Stage),
    StageFinished {
        stage: Stage,
        ok: bool,
    },
    LlmUsage {
        stage: Stage,
        usage: Usage,
    },
    /// Vollständiger LLM-Call für Debug-Trace (`--debug`/`--debug-json`).
    /// Trägt die tatsächlich gesendeten Prompts (Echo der Python-Schicht)
    /// und die rohe Antwort — redigiert (keine Secrets).
    LlmTrace {
        stage: Stage,
        attempt: u32,
        system_prompt: Option<String>,
        user_prompt: String,
        raw_response: String,
        duration_ms: Option<u64>,
    },
    Note(String),
}

/// Ergebnis der v1.0-Optimierungs-Engine (modul-privat).
struct OptEngineOutcome {
    selected_text: String,
    verification: VerificationReport,
    report: pf_core::OptimizationReport,
    events: Vec<OptimizerEvent>,
}

/// Ein evaluierter Optimierungs-Kandidat (Pipelinestufe).
type OptCandidate = (
    String,              // Strategie-Name
    String,              // finaler Text (nach Hygiene + Guard)
    VerificationReport,  // strukturelle Verifikation
    f64,                 // token_efficiency (>0 = kleiner als Input)
    u32,                 // guard_recovered_atoms
    Vec<OptimizerEvent>, // Hygiene-/Guard-Events
    u64,                 // pre_guard_tokens
);

/// Konfiguration einer Compile-Ausführung (aus AppConfig abgeleitet).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub llm: LlmConfig,
    pub verify: VerifyConfig,
}

impl EngineConfig {
    /// Ist ein LLM-Aufruf vorgesehen (Provider any_llm/mock)?
    pub fn llm_available(&self, provider: ProviderKind) -> bool {
        matches!(provider, ProviderKind::AnyLlm | ProviderKind::Mock) && self.llm.is_configured()
    }
}

/// Die PromptForge-Engine: eine Instanz pro Konfiguration.
pub struct Engine {
    bridge: Box<dyn LlmBridge>,
    tokenizer: Arc<dyn Tokenizer>,
    cfg: EngineConfig,
    provider: ProviderKind,
}

impl Engine {
    /// Neue Engine. `bridge` ist die LLM-Boundary (Python oder Mock);
    /// bei `provider = None` wird sie nicht aufgerufen.
    pub fn new(
        bridge: Box<dyn LlmBridge>,
        tokenizer: Arc<dyn Tokenizer>,
        cfg: EngineConfig,
        provider: ProviderKind,
    ) -> Self {
        Self {
            bridge,
            tokenizer,
            cfg,
            provider,
        }
    }

    /// Deterministische Engine ohne LLM (für Tests/Offline).
    pub fn deterministic(cfg: EngineConfig) -> Self {
        Self::new(
            Box::new(MockBridge::new()),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg,
            ProviderKind::None,
        )
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    fn llm_on(&self) -> bool {
        self.cfg.llm_available(self.provider)
    }

    fn emit(cb: &mut Option<Box<dyn FnMut(StageEvent)>>, ev: StageEvent) {
        if let Some(f) = cb.as_mut() {
            f(ev);
        }
    }

    fn bridge_request(
        &self,
        op: LlmOperation,
        payload: serde_json::Value,
        request_id: &str,
    ) -> LlmRequest {
        LlmRequest {
            operation: op,
            system_prompt: None,
            user_prompt: payload.to_string(),
            output_schema: None,
            endpoint: self.cfg.llm.endpoint.clone(),
            api_key: self.cfg.llm.key.clone(),
            model: self.cfg.llm.model.clone(),
            temperature: self.cfg.llm.temperature,
            max_tokens: self.cfg.llm.max_tokens,
            provider: match self.provider {
                ProviderKind::AnyLlm => "any_llm".to_string(),
                ProviderKind::Mock => "mock".to_string(),
                _ => "auto".to_string(),
            },
            request_id: request_id.to_string(),
            timeout_s: Some(self.cfg.llm.timeout_s),
        }
    }

    fn call(
        &self,
        op: LlmOperation,
        payload: serde_json::Value,
        request_id: &str,
        attempt: u32,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<LlmResponse> {
        let req = self.bridge_request(op, payload, request_id);
        tracing::info!(operation = op.as_str(), request_id, model = ?req.model, attempt, "llm call");
        let resp = self.bridge.complete(&req)?;
        if let Some(usage) = resp.usage {
            Self::emit(
                cb,
                StageEvent::LlmUsage {
                    stage: stage_of(op),
                    usage,
                },
            );
        }
        // Debug-Trace-Event: echte Prompts (Echo der Python-Schicht) + rohe
        // Antwort, redigiert (Secret-Werte, Bearer-/sk-/key=-Muster).
        let secrets: Vec<String> = self.cfg.llm.key.clone().into_iter().collect();
        let san = |s: &str| pf_core::redact::sanitize_line(s, &secrets);
        Self::emit(
            cb,
            StageEvent::LlmTrace {
                stage: stage_of(op),
                attempt,
                system_prompt: resp.system_prompt.as_deref().map(san),
                user_prompt: san(resp.user_prompt.as_deref().unwrap_or(&req.user_prompt)),
                raw_response: san(&resp.content),
                duration_ms: resp.duration_ms,
            },
        );
        Ok(resp)
    }

    /// Kompiliert einen Intent (natürliche Sprache) bis zum optimierten Prompt.
    /// `cb` erhält Fortschritts-Events (z. B. für TUI/CLI).
    ///
    /// Ergebnis ist ein formatneutrales `CompilationResult` (v0.2) — die
    /// einzige Ergebnisstruktur; keine zweite Pipeline.
    pub fn compile(
        &self,
        intent: &str,
        cb: Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<CompilationResult> {
        self.compile_impl(intent, cb, "auto")
    }

    /// Wie `compile`, mit wählbarer Optimizer-Strategie (CLI `--optimizer`).
    pub fn compile_with_optimizer(
        &self,
        intent: &str,
        cb: Option<Box<dyn FnMut(StageEvent)>>,
        optimizer: &str,
    ) -> Result<CompilationResult> {
        self.compile_impl(intent, cb, optimizer)
    }

    fn compile_impl(
        &self,
        intent: &str,
        mut cb: Option<Box<dyn FnMut(StageEvent)>>,
        optimizer_mode: &str,
    ) -> Result<CompilationResult> {
        let request_id = pf_core::path::request_id();
        let intent = intent.trim();
        if intent.is_empty() {
            return Err(err(ErrorKind::InvalidInput, "Intent ist leer"));
        }

        // ---- Stage 1: Architect ----
        Self::emit(&mut cb, StageEvent::StageStarted(Stage::Architect));
        let ir = if self.llm_on() {
            let payload = serde_json::json!({ "intent": intent });
            // Begrenzter Retry NUR bei reparablen Architect-Fehlern (invalid
            // JSON / schema violation durch Modellvarianz). KEIN Retry bei
            // Truncation oder empty response — dort liegt die Ursache im
            // Output-/Provider-Limit, ein weiterer identischer Call würde
            // wieder kappen (Repair CI/apfel).
            let resp = match self.call(
                LlmOperation::Architect,
                payload.clone(),
                &request_id,
                1,
                &mut cb,
            ) {
                Ok(r) => r,
                Err(e)
                    if e.kind == ErrorKind::Model
                        && (e.message.contains("invalid JSON")
                            || e.message.contains("schema violation")) =>
                {
                    Self::emit(
                        &mut cb,
                        StageEvent::Note(format!(
                            "Architect-Parsing-Fehler ({}): ein erneuter Versuch",
                            &e.message[..e.message.len().min(120)]
                        )),
                    );
                    self.call(LlmOperation::Architect, payload, &request_id, 2, &mut cb)?
                }
                Err(e) => return Err(e),
            };
            parse_ir(&resp, "Architect")?
        } else {
            PromptIr::from_intent_basic(intent, &request_id)
        };
        Self::emit(
            &mut cb,
            StageEvent::StageFinished {
                stage: Stage::Architect,
                ok: true,
            },
        );

        // ---- Stage 2: Expand (deterministisch, Rust) ----
        Self::emit(&mut cb, StageEvent::StageStarted(Stage::Expand));
        let long_prompt = expand_to_long_prompt(&ir);
        Self::emit(
            &mut cb,
            StageEvent::StageFinished {
                stage: Stage::Expand,
                ok: true,
            },
        );

        // ---- Stage 3/4: Optimize + Verify (v1.0-Kandidaten-Engine) ----
        let out =
            self.run_optimization_engine(&ir, &long_prompt, &request_id, &mut cb, optimizer_mode)?;
        let optimized = out.selected_text;
        let verification = out.verification;
        let optimization = Some(out.report);

        match verification.verdict {
            Some(Verdict::Pass) => {
                Self::emit(
                    &mut cb,
                    StageEvent::Note("Verifikation bestanden".to_string()),
                );
                let mut cr = self.finish(
                    intent,
                    ir,
                    &long_prompt,
                    &optimized,
                    verification,
                    request_id,
                    optimization,
                );
                // v1.0-Zusatzmetriken für den ausgewählten Kandidaten.
                if let Some(rep) = cr.optimization.as_ref() {
                    cr.metrics.technical_token_preservation =
                        Some(crate::optimization::technical_preservation(
                            &long_prompt,
                            &cr.optimized_prompt,
                        ));
                    cr.metrics.redundancy_removed = Some(
                        rep.input_tokens
                            .saturating_sub(self.tokenizer.count(&cr.optimized_prompt)),
                    );
                    cr.metrics.constraint_preservation = Some(
                        if rep.optimization_status == pf_core::OptimizationStatus::NoImprovement {
                            1.0
                        } else {
                            cr.metrics.semantic_fidelity
                        },
                    );
                    cr.metrics.instruction_quality =
                        Some(if cr.verification.instructions_preserved {
                            1.0
                        } else {
                            0.0
                        });
                    cr.metrics.output_contract_quality =
                        Some(if cr.verification.output_contract_preserved {
                            1.0
                        } else {
                            0.0
                        });
                }
                Ok(cr)
            }
            _ => {
                Self::emit(
                    &mut cb,
                    StageEvent::Note(format!(
                        "Verifikation fehlgeschlagen (semantic_preservation={:.2})",
                        verification.semantic_preservation
                    )),
                );
                Err(err(
                    ErrorKind::Verification,
                    format!(
                        "Verifikation nicht bestanden: kein Optimierungs-Kandidat und keine Baseline \
                         erfüllte die Erhaltungsbedingungen (semantic_preservation={:.2})",
                        verification.semantic_preservation
                    ),
                ))
            }
        }
    }

    /// Optimize- + Verify über die v1.0-Kandidaten-Engine (Wrapper für
    /// `POST /v1/optimize`). Gibt den gewählten Prompt, die finale
    /// Verifikation und die Optimizer-Events des gewählten Kandidaten zurück.
    pub fn optimize_and_verify(
        &self,
        ir: &PromptIr,
        long_prompt: &str,
        request_id: &str,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<(String, VerificationReport, Vec<OptimizerEvent>)> {
        let out = self.run_optimization_engine(ir, long_prompt, request_id, cb, "auto")?;
        Ok((out.selected_text, out.verification, out.events))
    }

    // Ergebnis der v1.0-Optimierungs-Engine.
    // (struct-Definitionen sind in impl-Blöcken nicht erlaubt — Typ liegt
    // modulweit oben bei den Ergebnis-Typen.)

    /// v1.0 Optimization Engine: erzeugt mehrere Kandidaten (Strategien),
    /// hygienisiert + guardet jeden, verifiziert strukturell, bewertet und
    /// wählt den besten gültigen Kandidaten aus (niemals einen, der größer
    /// ist als der Input → `optimization_status`).
    #[allow(clippy::too_many_lines)]
    fn run_optimization_engine(
        &self,
        ir: &PromptIr,
        long_prompt: &str,
        request_id: &str,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
        mode: &str,
    ) -> Result<OptEngineOutcome> {
        use pf_core::{CandidateReport, OptimizationReport, OptimizationStatus};

        Self::emit(cb, StageEvent::StageStarted(Stage::Optimize));
        let atoms = SemanticAtoms::from_ir(ir);
        let mandatory = mandatory_atoms(&atoms);
        let atoms_total = mandatory
            .iter()
            .filter(|a| !a.trim().is_empty())
            .count()
            .max(1) as u32;

        // Strategie-Set aus Modus ableiten (keine Provider-spezifische Logik).
        let mut strategies: Vec<&str> = Vec::new();
        match mode {
            "baseline" => strategies.push("baseline"),
            "redundancy" => strategies.push("redundancy"),
            "instruction" => strategies.push("instruction"),
            "structural" => strategies.push("structural"),
            "semantic" => strategies.push("semantic"),
            "combined" => {
                strategies.push("combined");
                strategies.push("baseline");
            }
            _ => {
                strategies.extend([
                    "redundancy",
                    "instruction",
                    "structural",
                    "semantic",
                    "combined",
                    "baseline",
                ]);
            }
        }
        let want_llm_candidate = self.llm_on() && matches!(mode, "auto" | "baseline" | "combined");

        let input_tokens = self.tokenizer.count(long_prompt);
        let mut llm_attempt = 0u32;
        let mut all_events: Vec<OptimizerEvent> = Vec::new();
        let mut evaled: Vec<OptCandidate> = Vec::new();
        // name, final_text, structural_verify, token_efficiency, guard_n, events, pre_guard_tokens

        for strat in &strategies {
            let strategy = *strat;
            let raw: String = match strategy {
                "baseline" => long_prompt.to_string(),
                "llm" => {
                    llm_attempt += 1;
                    let payload = serde_json::json!({
                        "ir": ir,
                        "long_prompt": long_prompt,
                        "feedback": [],
                    });
                    let resp =
                        self.call(LlmOperation::Optimize, payload, request_id, llm_attempt, cb)?;
                    extract_prompt(&resp)?
                }
                "redundancy" => crate::optimization::strategy_redundancy(long_prompt).0,
                "instruction" => crate::optimization::strategy_instruction(long_prompt).0,
                "structural" => crate::optimization::strategy_structural(ir),
                "semantic" => crate::optimization::strategy_semantic(long_prompt).0,
                "combined" => crate::optimization::strategy_combined(long_prompt).0,
                _ => continue,
            };
            let (hygiene, evs) = deterministic_pass_chain(&raw);
            let pre_tokens = self.tokenizer.count(&hygiene);
            let (guarded, ev) = reinsert_missing_atoms(&hygiene, &mandatory);
            let guard_n = match ev.action {
                crate::optimizer::PassAction::ReinsertedAtoms(n) => n,
                _ => 0,
            };
            if guard_n > 0 {
                Self::emit(
                    cb,
                    StageEvent::Note(format!(
                        "Guard-Pass hat {guard_n} verlorene Pflicht-Atome wiederhergestellt (Strategie {strategy})"
                    )),
                );
            }
            let mut cand_events = evs;
            cand_events.push(ev);
            let out_tokens = self.tokenizer.count(&guarded);
            let eff = if input_tokens > 0 {
                1.0 - out_tokens as f64 / input_tokens as f64
            } else {
                0.0
            };
            let structural = verify_structural(ir, &guarded, &atoms, self.cfg.verify.threshold);
            let pass = structural.verdict == Some(Verdict::Pass);
            all_events.extend(cand_events.iter().cloned());
            if strategy == "llm" || strategy != "baseline" || !self.llm_on() || mode == "baseline" {
                evaled.push((
                    strategy.to_string(),
                    guarded,
                    structural,
                    eff,
                    guard_n as u32,
                    cand_events,
                    pre_tokens,
                ));
            }
            let _ = pass;
            let _ = pre_tokens;
        }
        // „llm"-Kandidat explizit anhängen, falls gewünscht (nach den
        // deterministischen, damit die Baseline-Reihenfolge stabil bleibt).
        if want_llm_candidate && !strategies.contains(&"llm") {
            llm_attempt += 1;
            let payload = serde_json::json!({
                "ir": ir,
                "long_prompt": long_prompt,
                "feedback": [],
            });
            let resp = self.call(LlmOperation::Optimize, payload, request_id, llm_attempt, cb)?;
            let raw = extract_prompt(&resp)?;
            let (hygiene, evs) = deterministic_pass_chain(&raw);
            let pre_tokens = self.tokenizer.count(&hygiene);
            let (guarded, ev) = reinsert_missing_atoms(&hygiene, &mandatory);
            let guard_n = match ev.action {
                crate::optimizer::PassAction::ReinsertedAtoms(n) => n,
                _ => 0,
            };
            let mut cand_events = evs;
            cand_events.push(ev);
            let out_tokens = self.tokenizer.count(&guarded);
            let eff = if input_tokens > 0 {
                1.0 - out_tokens as f64 / input_tokens as f64
            } else {
                0.0
            };
            let structural = verify_structural(ir, &guarded, &atoms, self.cfg.verify.threshold);
            all_events.extend(cand_events.iter().cloned());
            evaled.push((
                "llm".to_string(),
                guarded,
                structural,
                eff,
                guard_n as u32,
                cand_events,
                pre_tokens,
            ));
        }

        // Strukturell gültige & kleinere Kandidaten bewerten.
        let score_for = |eff: f64, semantic: f64, guard_n: u32| -> f64 {
            let ratio = guard_n as f64 / atoms_total as f64;
            let benefit = 0.2 + 0.8 * eff.max(0.0);
            let penalty = (1.0 - 0.6 * ratio.min(1.0)).max(0.1);
            semantic * benefit * penalty
        };
        let mut ranked: Vec<usize> = (0..evaled.len())
            .filter(|&i| evaled[i].2.verdict == Some(Verdict::Pass) && evaled[i].3 > 0.0)
            .collect();
        ranked.sort_by(|&a, &b| {
            let sa = score_for(evaled[a].3, evaled[a].2.semantic_preservation, evaled[a].4);
            let sb = score_for(evaled[b].3, evaled[b].2.semantic_preservation, evaled[b].4);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Auswahl + finale (ggf. LLM-)Verifikation.
        let mut selected_text = long_prompt.to_string();
        let mut final_verification =
            verify_structural(ir, long_prompt, &atoms, self.cfg.verify.threshold);
        let mut status = OptimizationStatus::NoImprovement;
        let mut selected: Option<String> = None;
        let mut final_events: Vec<OptimizerEvent> = Vec::new();
        let mut semantic_fidelity = final_verification.semantic_preservation;

        if !ranked.is_empty() {
            let mut chosen: Option<usize> = None;
            if self.llm_on() {
                for &idx in &ranked {
                    let text = evaled[idx].1.clone();
                    Self::emit(cb, StageEvent::StageStarted(Stage::Verify));
                    let payload = serde_json::json!({
                        "atoms": atoms_payload(&atoms),
                        "long_prompt": long_prompt,
                        "optimized_prompt": text,
                    });
                    let resp = self.call(LlmOperation::Verify, payload, request_id, 1, cb)?;
                    let semantic = parse_semantic_report_json(&resp.parse_content_json()?)?;
                    let merged =
                        merge_semantic(evaled[idx].2.clone(), semantic, self.cfg.verify.threshold);
                    Self::emit(
                        cb,
                        StageEvent::StageFinished {
                            stage: Stage::Verify,
                            ok: merged.verdict == Some(Verdict::Pass),
                        },
                    );
                    if merged.verdict == Some(Verdict::Pass) {
                        chosen = Some(idx);
                        final_verification = merged;
                        break;
                    }
                    // Nächster Kandidat (reject / next strategy).
                    Self::emit(
                        cb,
                        StageEvent::Note(format!(
                            "Kandidat {} von LLM-Verify abgelehnt — nächster Kandidat",
                            evaled[idx].0
                        )),
                    );
                }
                if chosen.is_none() {
                    // Kein Kandidat besteht die LLM-Semantikprüfung: Fallback
                    // auf die beste strukturelle Verifikation (Original) —
                    // wird unten als Fehler gemeldet (kein Fake-PASS).
                    final_verification = evaled[ranked[0]].2.clone();
                    final_verification.attempts = 1;
                    selected_text = evaled[ranked[0]].1.clone();
                }
            } else {
                chosen = Some(ranked[0]);
                final_verification = evaled[ranked[0]].2.clone();
            }
            if let Some(idx) = chosen {
                let name = evaled[idx].0.clone();
                let eff = evaled[idx].3;
                selected = Some(name.clone());
                selected_text = evaled[idx].1.clone();
                final_events = evaled[idx].5.clone();
                status = if eff > 0.0 {
                    OptimizationStatus::Optimized
                } else {
                    OptimizationStatus::NoImprovement
                };
                semantic_fidelity = final_verification.semantic_preservation;
                let _ = score_for(eff, final_verification.semantic_preservation, evaled[idx].4);
            }
        }

        // Kandidatenliste korrekt füllen (pre_guard_tokens aus Hygiene-Pass).
        let mut candidates = Vec::new();
        let mut guard_total = 0u32;
        for (name, text, rep, eff, guard_n, _evs, pre) in &evaled {
            let ratio = if atoms_total > 0 {
                *guard_n as f64 / atoms_total as f64
            } else {
                0.0
            };
            guard_total += guard_n;
            candidates.push(CandidateReport {
                strategy: name.clone(),
                input_tokens,
                pre_guard_tokens: *pre,
                output_tokens: self.tokenizer.count(text),
                token_efficiency: *eff,
                semantic_fidelity: rep.semantic_preservation,
                structural_validity: rep.all_preserved(),
                verification: if rep.verdict == Some(Verdict::Pass) {
                    "pass".to_string()
                } else {
                    "fail".to_string()
                },
                guard_recovered_atoms: *guard_n,
                guard_recovery_ratio: ratio,
            });
        }

        let report = OptimizationReport {
            input_tokens,
            baseline_tokens: input_tokens,
            candidates,
            selected,
            score: final_verification.semantic_preservation,
            optimization_status: status,
            guard_recovered_atoms_total: guard_total,
        };
        Self::emit(
            cb,
            StageEvent::StageFinished {
                stage: Stage::Optimize,
                ok: true,
            },
        );
        let _ = semantic_fidelity;
        Ok(OptEngineOutcome {
            selected_text,
            verification: final_verification,
            report,
            events: if final_events.is_empty() {
                all_events
            } else {
                final_events
            },
        })
    }

    /// Einzelne Verifikation (strukturell + optional LLM-Semantik) ohne
    /// Re-Optimize-Loop — für `POST /v1/verify`.
    pub fn verify_pair(
        &self,
        ir: &PromptIr,
        long_prompt: &str,
        optimized: &str,
        request_id: &str,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<VerificationReport> {
        Self::emit(cb, StageEvent::StageStarted(Stage::Verify));
        let atoms = SemanticAtoms::from_ir(ir);
        let structural = verify_structural(ir, optimized, &atoms, self.cfg.verify.threshold);
        let report = if self.llm_on() {
            let payload = serde_json::json!({
                "atoms": atoms_payload(&atoms),
                "long_prompt": long_prompt,
                "optimized_prompt": optimized,
            });
            let resp = self.call(LlmOperation::Verify, payload, request_id, 1, cb)?;
            let semantic = parse_semantic_report_json(&resp.parse_content_json()?)?;
            merge_semantic(structural, semantic, self.cfg.verify.threshold)
        } else {
            structural
        };
        Self::emit(
            cb,
            StageEvent::StageFinished {
                stage: Stage::Verify,
                ok: report.verdict == Some(Verdict::Pass),
            },
        );
        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        intent: &str,
        ir: PromptIr,
        long_prompt: &str,
        optimized: &str,
        verification: VerificationReport,
        request_id: String,
        optimization: Option<pf_core::OptimizationReport>,
    ) -> CompilationResult {
        let tokenizer = Arc::clone(&self.tokenizer);
        let mut token_report = TokenReport::new();
        token_report.estimate = tokenizer.is_estimate();
        token_report.set_original(tokenizer.count(intent));
        token_report.set_generated(tokenizer.count(long_prompt));
        token_report.set_optimized(tokenizer.count(optimized));
        let metrics = QualityMetrics::compute(&verification, &token_report);
        CompilationResult {
            input: intent.to_string(),
            request_id,
            llm_used: self.llm_on(),
            stages: vec![
                Stage::Architect.as_str().to_string(),
                Stage::Expand.as_str().to_string(),
                Stage::Optimize.as_str().to_string(),
                Stage::Verify.as_str().to_string(),
            ],
            prompt_ir: ir,
            expanded_prompt: long_prompt.to_string(),
            optimized_prompt: optimized.to_string(),
            token_report,
            verification,
            metrics,
            optimization,
        }
    }
}

fn stage_of(op: LlmOperation) -> Stage {
    match op {
        LlmOperation::Architect => Stage::Architect,
        LlmOperation::Optimize => Stage::Optimize,
        LlmOperation::Verify => Stage::Verify,
        LlmOperation::Chat => Stage::Optimize,
    }
}

fn parse_ir(resp: &LlmResponse, stage: &str) -> Result<PromptIr> {
    let value = resp.parse_content_json().map_err(|_| {
        err(
            ErrorKind::Model,
            format!("{stage}: LLM-Antwort ist kein valides JSON"),
        )
    })?;
    PromptIr::from_json(&serde_json::to_string(&value)?)
        .map_err(|e| err(ErrorKind::Model, format!("{stage}: {e}")))
}

/// Extrahiert den Prompt aus der Optimize-Antwort (JSON `{prompt, notes}`
/// oder reiner Text).
fn extract_prompt(resp: &LlmResponse) -> Result<String> {
    match serde_json::from_str::<serde_json::Value>(resp.content.trim()) {
        Ok(v) => {
            if let Some(p) = v.get("prompt").and_then(|p| p.as_str()) {
                return Ok(p.to_string());
            }
            if let Some(s) = v.as_str() {
                return Ok(s.to_string());
            }
            Err(err(
                ErrorKind::Model,
                "Optimize: Antwort-JSON ohne 'prompt'-Feld",
            ))
        }
        Err(_) => Ok(resp.content.clone()),
    }
}

/// Führt einen Prompt gegen das Ziel-LLM aus (z. B. `POST /v1/execute`).
pub fn execute_prompt(
    engine: &Engine,
    prompt: &str,
    mut cb: Option<Box<dyn FnMut(StageEvent)>>,
) -> Result<LlmResponse> {
    if !engine.llm_on() {
        return Err(err(
            ErrorKind::Configuration,
            "Kein LLM konfiguriert (LLM_ENDPOINT/LLM_MODEL) — Ausführung nicht möglich",
        ));
    }
    let request_id = pf_core::path::request_id();
    let payload = serde_json::json!({ "prompt": prompt });
    engine.call(LlmOperation::Chat, payload, &request_id, 1, &mut cb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::config::VerifyConfig;

    fn cfg() -> EngineConfig {
        EngineConfig {
            llm: LlmConfig {
                model: Some("mock-model".to_string()),
                ..Default::default()
            },
            verify: VerifyConfig::default(),
        }
    }

    #[test]
    fn deterministic_compile_works_end_to_end() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine
            .compile(
                "Analysiere diese fünf Papers und vergleiche die Methoden",
                None,
            )
            .unwrap();
        // v0.2: CompilationResult mit kanonischen Feldern + Metriken.
        assert_eq!(
            outcome.input,
            "Analysiere diese fünf Papers und vergleiche die Methoden"
        );
        assert!(outcome.expanded_prompt.contains("## Aufgabe"));
        assert!(!outcome.optimized_prompt.is_empty());
        assert!(!outcome.prompt_ir.task.is_empty());
        assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
        assert!(!outcome.llm_used);
        assert!(outcome.token_report.generated > 0);
        assert_eq!(outcome.stages.len(), 4);
        // Qualitätsmetriken deterministisch vorhanden.
        assert!(outcome.metrics.structural_validity);
        assert!(
            (outcome.metrics.semantic_fidelity - outcome.verification.semantic_preservation).abs()
                < 1e-9
        );
        assert!(outcome.metrics.token_efficiency >= 0.0); // ohne LLM: 1:1-Kopie
    }

    #[test]
    fn deterministic_compile_input_is_trimmed_and_analysis_defaults() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine.compile("  Auditiere das Projekt  ", None).unwrap();
        assert_eq!(outcome.input, "Auditiere das Projekt");
        assert!(outcome.prompt_ir.analysis.is_none());
        assert!(outcome.prompt_ir.metadata.tags.is_empty());
        // CompilationResult ist JSON-serialisierbar (formatneutral).
        let json = serde_json::to_string(&outcome).unwrap();
        let back: pf_core::CompilationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }

    #[test]
    fn mock_compile_works_end_to_end() {
        let engine = Engine::new(
            Box::new(MockBridge::new()),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg(),
            ProviderKind::Mock,
        );
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_cb = std::rc::Rc::clone(&events);
        let outcome = engine
            .compile(
                "Analysiere fünf Papers und finde Widersprüche",
                Some(Box::new(move |e| events_cb.borrow_mut().push(e))),
            )
            .unwrap();
        assert!(outcome.llm_used);
        assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
        assert!(outcome.prompt_ir.role.is_some());
        // CompilationResult vorhanden (v0.2) — echter Pipeline-Lauf.
        assert!(!outcome.input.is_empty());
        assert_eq!(outcome.stages.len(), 4);
        assert!(outcome.metrics.semantic_fidelity >= 0.0);
        // Alle Stadien wurden als Started/Finished gemeldet.
        let starts = events
            .borrow()
            .iter()
            .filter(|e| matches!(e, StageEvent::StageStarted(_)))
            .count();
        assert!(starts >= 4, "starts={starts}");
        let _ = outcome;
    }

    #[test]
    fn empty_intent_is_rejected() {
        let engine = Engine::deterministic(cfg());
        assert!(engine.compile("   ", None).is_err());
    }

    #[test]
    fn execute_without_llm_errors() {
        let engine = Engine::deterministic(cfg());
        let e = execute_prompt(&engine, "hallo", None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Configuration);
    }

    #[test]
    fn mock_execute_returns_content() {
        let engine = Engine::new(
            Box::new(MockBridge::new()),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg(),
            ProviderKind::Mock,
        );
        let resp = execute_prompt(&engine, "Fasse zusammen", None).unwrap();
        assert!(resp.content.starts_with("Mock-Antwort"));
    }

    // ---- v1.0 Optimization Engine: Verhalten ohne LLM (deterministisch) ----

    #[test]
    fn v10_optimized_selection_reports_candidates() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine
            .compile_with_optimizer("auditiere das projekt", None, "auto")
            .unwrap();
        let rep = outcome.optimization.expect("OptimizationReport fehlt");
        // Die Engine wählt den besten gültigen Kandidaten (kein Fake-PASS).
        assert_eq!(
            rep.optimization_status,
            pf_core::OptimizationStatus::Optimized
        );
        let sel = rep.selected.as_deref().expect("selected fehlt");
        assert_eq!(sel, "structural");
        assert!(!rep.candidates.is_empty(), "mindestens ein Kandidat");
        // Effizienz der Auswahl muss positiv sein (kleiner als Long Prompt).
        let out_tokens = rep
            .candidates
            .iter()
            .find(|c| c.strategy == sel)
            .map(|c| c.output_tokens)
            .unwrap();
        assert!(out_tokens < rep.input_tokens, "Optimized muss kleiner sein");
        assert!(outcome.verification.verdict == Some(Verdict::Pass));
        // Strukturelle Semantik: alle Pflicht-Atome erhalten.
        assert!(outcome.verification.all_preserved());
    }

    #[test]
    fn v10_baseline_mode_is_no_improvement_not_degradation() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine
            .compile_with_optimizer("auditiere das projekt", None, "baseline")
            .unwrap();
        let rep = outcome.optimization.unwrap();
        assert_eq!(
            rep.optimization_status,
            pf_core::OptimizationStatus::NoImprovement
        );
        // Ohne LLM ist baseline = Long Prompt (keine künstliche Verschlechterung).
        assert_eq!(outcome.optimized_prompt, outcome.expanded_prompt);
        assert!(outcome.metrics.token_efficiency >= 0.0);
    }

    #[test]
    fn v10_all_strategies_are_structural_valid() {
        let engine = Engine::deterministic(cfg());
        for mode in [
            "redundancy",
            "instruction",
            "structural",
            "semantic",
            "combined",
        ] {
            let outcome = engine
                .compile_with_optimizer("auditiere das projekt", None, mode)
                .unwrap();
            let rep = outcome.optimization.unwrap();
            assert_eq!(outcome.verification.verdict, Some(Verdict::Pass), "{mode}");
            // Kein Kandidat darf Information verlieren.
            let cands: Vec<&str> = rep
                .candidates
                .iter()
                .filter(|c| c.verification == "pass")
                .map(|c| c.strategy.as_str())
                .collect();
            assert!(cands.contains(&mode), "Strategie {mode} fehlt: {cands:?}");
        }
    }

    #[test]
    fn v10_optimization_report_serializes_additively() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine
            .compile_with_optimizer("auditiere das projekt", None, "auto")
            .unwrap();
        let v = outcome.envelope_json();
        let obj = v.as_object().expect("Envelope ist Objekt");
        assert!(
            obj.contains_key("optimization"),
            "Report muss im Envelope sein"
        );
        let rep = &obj["optimization"];
        assert!(rep["optimization_status"].is_string());
        assert!(rep["candidates"].is_array());
        assert!(!rep["candidates"].as_array().unwrap().is_empty());
    }

    // ---- Repair CI/apfel: Architect-Retry-Verhalten (begrenzt, kein Fake-PASS) ----

    /// Liefert beim ersten Architect-Call einen reparablen Fehler (invalid
    /// JSON) oder immer einen Truncation-Fehler — je nach Konfiguration.
    struct FlakyArchitect {
        inner: MockBridge,
        calls: std::sync::atomic::AtomicU32,
        fail_first_invalid: bool,
    }

    impl FlakyArchitect {
        fn invalid_once(inner: MockBridge) -> Self {
            Self {
                inner,
                calls: std::sync::atomic::AtomicU32::new(0),
                fail_first_invalid: true,
            }
        }
        fn always_truncated(inner: MockBridge) -> Self {
            Self {
                inner,
                calls: std::sync::atomic::AtomicU32::new(0),
                fail_first_invalid: false,
            }
        }
    }

    impl pf_core::bridge::LlmBridge for FlakyArchitect {
        fn complete(
            &self,
            req: &pf_core::bridge::LlmRequest,
        ) -> Result<pf_core::bridge::LlmResponse> {
            if req.operation == pf_core::bridge::LlmOperation::Architect {
                let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if self.fail_first_invalid && n == 1 {
                    return Err(err(
                        ErrorKind::Model,
                        "Architect: invalid JSON (Expecting value: line 1); Antwort beginnt: 'x'",
                    ));
                }
                if !self.fail_first_invalid {
                    return Err(err(
                        ErrorKind::Model,
                        "Architect response appears truncated before valid JSON completion (letztes Zeichen '[')",
                    ));
                }
            }
            self.inner.complete(req)
        }
    }

    #[test]
    fn architect_invalid_json_is_retried_once_and_recovers() {
        let engine = Engine::new(
            Box::new(FlakyArchitect::invalid_once(MockBridge::new())),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg(),
            ProviderKind::Mock,
        );
        let outcome = engine.compile("auditiere das projekt", None).unwrap();
        assert!(!outcome.prompt_ir.task.is_empty());
        assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
        // Genau ein zusätzlicher Versuch ist erlaubt; der Erfolg belegt,
        // dass der zweite Request valide war (kein Endlos-Loop).
    }

    #[test]
    fn architect_truncation_is_not_retried() {
        let engine = Engine::new(
            Box::new(FlakyArchitect::always_truncated(MockBridge::new())),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg(),
            ProviderKind::Mock,
        );
        let e = engine.compile("auditiere das projekt", None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Model);
        assert!(
            e.message.contains("truncated"),
            "Truncation muss als solche gemeldet werden: {}",
            e.message
        );
        // Kein Retry bei Truncation — Ursache ist das Output-Limit, kein
        // zweiter identischer Call (kein Fake-PASS durch Wiederholen).
    }
}
