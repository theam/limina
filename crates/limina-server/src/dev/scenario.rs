//! The demo scenario: a retrieval study that confirms two hypotheses and asks the human to choose
//! an operating point. The texts and numbers are illustrative, not measured.
//!
//! Each turn picks its step from the project's artifacts, so the scenario resumes wherever the
//! project is:
//!
//! 1. no `H001`: establish the baseline and propose `H001` and `H002`;
//! 2. no `F001`: design, claim, observe and complete `E001`, publish `F001`, confirm `H001`;
//! 3. no `E002`: sweep the grounding threshold, design `E002` and ask which threshold to use;
//! 4. no `F002`: run `E002` with the answer, publish `F002`, confirm `H002`;
//! 5. afterwards: report status and wait for direction.

use limina_core::Result;
use limina_engines::RuntimeDecision;
use limina_engines::turn::{MissionDecision, MissionUpdate};
use serde_json::{Value, json};

use super::agent::{Turn, decision};

const OBJECTIVE: &str = "Reduce unsupported citations without hurting answer rate";

/// `artifact.get("version", 1)`.
fn version(artifact: &Value) -> Value {
    artifact.get("version").cloned().unwrap_or_else(|| json!(1))
}

impl Turn<'_> {
    pub(super) async fn play(&mut self) -> Result<RuntimeDecision> {
        let existing = self.artifact_ids().await?;
        let has = |id: &str| existing.iter().any(|existing| existing == id);
        let guidance = self.prompt.contains("New human guidance")
            && !self.prompt.contains("No new human guidance");
        if !has("H001") {
            return self.explore().await;
        }
        if !has("F001") {
            return self.first_experiment().await;
        }
        if !has("E002") {
            return self.ask_for_threshold().await;
        }
        if !has("F002") {
            return self.second_experiment().await;
        }
        self.follow_up(guidance).await
    }

    async fn explore(&mut self) -> Result<RuntimeDecision> {
        self.say(
            "Starting with the evaluation set and the current retrieval configuration so the \
             baseline is trustworthy before anything is optimized.",
        )
        .await?;
        self.run("ls data/eval && wc -l data/eval/questions.jsonl", 0.8, 0).await?;
        self.run("python scripts/inspect_config.py --component retriever", 1.0, 0).await?;
        self.run("python scripts/baseline.py --split dev --seed 7", 3.2, 0).await?;
        self.say(
            "Baseline on the 412-question dev split (BM25 top-8, no reranking):\n\n\
             | Metric | Value |\n|---|---|\n\
             | Answer rate | 91.3% |\n| Unsupported citations | 17.6% |\n| p95 latency | 1.9 s |\n\n\
             Most unsupported citations come from **near-duplicate passages** that share \
             vocabulary with the question but not the claim. That points at ranking, not \
             generation.",
        )
        .await?;
        self.knowledge(
            "hypotheses",
            json!({
                "title": "Cross-encoder reranking cuts unsupported citations",
                "statement": "Reranking BM25 top-50 with a cross-encoder and keeping the top 8 \
                    reduces unsupported citations by at least 30% relative, without lowering \
                    answer rate by more than 2 points.",
                "mechanism": "Lexical retrieval rewards term overlap; a cross-encoder scores \
                    question–passage entailment, demoting passages that share words but not the \
                    supporting claim.",
                "generalization": "The failure is lexical near-duplicates, which appear in any \
                    corpus with templated documentation, not just this help center.",
                "shortcut_risks": "Tuning the cut-off on the dev split; mistaking shorter answers \
                    for grounded ones.",
                "test_plan": "Compare against the BM25 baseline on the same 412 questions with a \
                    frozen generator.",
            }),
        )
        .await?;
        self.knowledge(
            "hypotheses",
            json!({
                "title": "Generation-time grounding check rejects weak citations",
                "statement": "Asking the generator to verify each citation against its passage \
                    and drop failures reduces unsupported citations further, at a measurable \
                    cost in answer rate.",
                "mechanism": "A second, claim-level check catches support failures that ranking \
                    cannot see.",
                "generalization": "Independent of the retriever, so it composes with H001.",
                "shortcut_risks": "Dropping citations instead of answers can hide unsupported \
                    claims.",
                "test_plan": "Run on top of the best H001 configuration; sweep the verification \
                    threshold.",
            }),
        )
        .await?;
        self.say(
            "I recorded two hypotheses. [[H001]] attacks the ranking failure directly; [[H002]] \
             adds a claim-level check on top. Testing H001 first because it is cheaper and \
             composes with H002.",
        )
        .await?;
        Ok(decision(
            "Established a trustworthy baseline and proposed H001 and H002.",
            "RUNNING",
            OBJECTIVE,
            "Design and run E001 to test H001 against the BM25 baseline",
            "None",
        ))
    }

    async fn first_experiment(&mut self) -> Result<RuntimeDecision> {
        let experiment = self
            .knowledge(
                "experiments",
                json!({
                    "hypothesis_id": "H001",
                    "title": "BM25 vs BM25 + cross-encoder rerank on dev",
                    "objective": "Measure unsupported citations and answer rate with reranking \
                        enabled.",
                    "procedure": "Retrieve top-50 with BM25, rerank with ms-marco-MiniLM-L-12, \
                        keep top 8. Same generator, prompts and seed as the baseline.",
                    "success_criteria": "≥30% relative reduction in unsupported citations; \
                        answer rate drop ≤2 points.",
                    "guardrails": "Frozen generator and prompt; report p95 latency; no dev-split \
                        threshold tuning.",
                }),
            )
            .await?;
        let experiment_id =
            experiment.get("id").and_then(Value::as_str).unwrap_or("E001").to_owned();
        self.knowledge(&format!("experiments/{experiment_id}/claim"), json!({"ttl_seconds": 1800}))
            .await?;
        self.edit(&["retrieval/pipeline.py", "configs/rerank.yaml"]).await?;
        self.run("pytest tests/retrieval -q", 1.6, 1).await?;
        self.say(
            "One test fails: the reranker returns scores in the wrong order for ties. Fixing the \
             comparator.",
        )
        .await?;
        self.edit(&["retrieval/rerank.py"]).await?;
        self.run("pytest tests/retrieval -q", 1.4, 0).await?;
        self.run(
            "python scripts/evaluate.py --split dev --config configs/rerank.yaml --seed 7",
            4.0,
            0,
        )
        .await?;
        let observations = format!("experiments/{experiment_id}/observations");
        self.knowledge(
            &observations,
            json!({
                "body": "Unsupported citations fell from 17.6% to 10.9% (−38% relative).",
                "evidence_ref": "runs/e001/metrics.json",
            }),
        )
        .await?;
        self.knowledge(
            &observations,
            json!({
                "body": "Answer rate 90.6% (−0.7 pts); p95 latency 2.4 s (+0.5 s).",
                "evidence_ref": "runs/e001/metrics.json",
            }),
        )
        .await?;
        let current = self.artifact(&experiment_id).await?;
        self.knowledge(
            &format!("experiments/{experiment_id}/complete"),
            json!({
                "results": "Unsupported citations 10.9% (baseline 17.6%). Answer rate 90.6% \
                    (baseline 91.3%). p95 latency 2.4 s (baseline 1.9 s).",
                "analysis": "The relative reduction (38%) clears the 30% bar with a 0.7-point \
                    answer-rate cost. Residual failures are mostly multi-hop questions where no \
                    single passage supports the claim.",
                "decision": "Adopt reranking as the new baseline; test H002 on top of it.",
                "expected_version": version(&current),
            }),
        )
        .await?;
        self.knowledge(
            "findings",
            json!({
                "experiment_id": experiment_id,
                "title": "Reranking removes most lexical near-duplicate citations",
                "finding": "A cross-encoder reranker over BM25 top-50 cuts unsupported citations \
                    by 38% relative at a 0.7-point answer-rate cost.",
                "evidence": "E001 on the 412-question dev split, frozen generator, seed 7.",
                "improvement": "Citations now point at passages that actually entail the claim \
                    for single-hop questions.",
                "remaining_debt": "Multi-hop questions remain the main failure; +0.5 s p95 \
                    latency.",
                "next_move": "Test the generation-time grounding check (H002) on top of \
                    reranking.",
                "impact": "HIGH",
            }),
        )
        .await?;
        let hypothesis = self.artifact("H001").await?;
        self.knowledge(
            "hypotheses/H001/decision",
            json!({
                "status": "CONFIRMED",
                "conclusion": "Supported by E001: −38% unsupported citations, −0.7 pts answer \
                    rate.",
                "expected_version": version(&hypothesis),
            }),
        )
        .await?;
        self.say(
            "**H001 is confirmed.** Reranking cut unsupported citations by 38% for a 0.7-point \
             answer-rate cost ([[F001]]). The remaining failures are multi-hop questions, which \
             is exactly what [[H002]] targets.",
        )
        .await?;
        Ok(decision(
            "Confirmed H001 with E001 and published F001.",
            "RUNNING",
            OBJECTIVE,
            "Test H002 (grounding check) on top of reranking",
            "None",
        ))
    }

    async fn ask_for_threshold(&mut self) -> Result<RuntimeDecision> {
        self.run(
            "python scripts/sweep_grounding.py --thresholds 0.5,0.6,0.7,0.8 --split dev",
            4.0,
            0,
        )
        .await?;
        self.say(
            "The grounding-check sweep shows a real trade-off:\n\n\
             | Threshold | Unsupported citations | Answer rate |\n|---|---|---|\n\
             | 0.5 | 8.1% | 90.2% |\n| 0.6 | 6.4% | 89.1% |\n| 0.7 | 4.9% | 87.4% |\n\
             | 0.8 | 3.8% | 84.0% |\n\n\
             Choosing the operating point is a product decision, so I’d rather not pick it for \
             you.",
        )
        .await?;
        self.knowledge(
            "experiments",
            json!({
                "hypothesis_id": "H002",
                "title": "Grounding check threshold on top of reranking",
                "objective": "Pick the verification threshold that best trades unsupported \
                    citations for answer rate.",
                "procedure": "Reranked retrieval (F001) + claim-level verification; evaluate the \
                    chosen threshold on the held-out test split.",
                "success_criteria": "Further ≥25% relative reduction vs F001 within the agreed \
                    answer-rate budget.",
                "guardrails": "Threshold chosen on dev, reported on test; no per-question \
                    overrides.",
            }),
        )
        .await?;
        let mut checkpoint = decision(
            "Swept the grounding threshold; the operating point needs a product decision.",
            "WAITING",
            OBJECTIVE,
            "Run E002 on the test split with the threshold you choose",
            "How much answer rate can we give up? At 0.6 we lose 1.5 pts for −41% unsupported \
             citations; at 0.7 we lose 3.2 pts for −55%.",
        );
        checkpoint.mission_update = Some(Box::new(MissionUpdate {
            headline: "Reranking helps. The next gain comes with an answer-rate trade-off.".into(),
            established: "On the dev split, reranking reduced unsupported citations from 17.6% to 10.9%, with a 0.7-point answer-rate cost. This is simulated demo evidence, not a live evaluation.".into(),
            ruled_out: "Reranking alone does not reach the mission's 8% citation target on dev. Stronger grounding thresholds reduce citations further but withhold more answers.".into(),
            uncertain: "The grounding threshold has not been confirmed on the held-out test split. The operating point still needs your direction; the sweep values are exploratory.".into(),
            evidence: vec!["F001".into(), "E002".into()],
            decision: Some(MissionDecision {
                question: "Keep the agreed budget of at most a two-point drop in answer rate?".into(),
                recommendation: "Keep that guardrail and evaluate threshold 0.6 on the held-out test split before considering rollout.".into(),
                trade_off: "A stricter threshold may remove more unsupported citations while leaving more questions unanswered. A dev result alone does not establish mission success.".into(),
            }),
        }));
        Ok(checkpoint)
    }

    async fn second_experiment(&mut self) -> Result<RuntimeDecision> {
        self.say(
            "Thanks — I’ll keep the answer-rate cost under 2 points, which means threshold \
             **0.6**, and confirm it on the held-out test split so the choice isn’t fitted to \
             dev.",
        )
        .await?;
        self.knowledge("experiments/E002/claim", json!({"ttl_seconds": 1800})).await?;
        self.edit(&["configs/grounding.yaml"]).await?;
        self.run(
            "python scripts/evaluate.py --split test --config configs/grounding.yaml --seed 7",
            4.0,
            0,
        )
        .await?;
        self.knowledge(
            "experiments/E002/observations",
            json!({
                "body": "Test split: unsupported citations 6.9% (F001 config: 11.4%), answer \
                    rate 89.4% (−1.6 pts).",
                "evidence_ref": "runs/e002/test-metrics.json",
            }),
        )
        .await?;
        let current = self.artifact("E002").await?;
        self.knowledge(
            "experiments/E002/complete",
            json!({
                "results": "Unsupported citations 6.9% vs 11.4% with reranking alone; answer \
                    rate 89.4%.",
                "analysis": "−39% relative on test, inside the 2-point budget. Dev and test \
                    agree within 0.5 pts, so the threshold is not overfitted.",
                "decision": "Ship threshold 0.6 behind the existing citation flag.",
                "expected_version": version(&current),
            }),
        )
        .await?;
        self.knowledge(
            "findings",
            json!({
                "experiment_id": "E002",
                "title": "A 0.6 grounding threshold halves remaining unsupported citations",
                "finding": "Claim-level verification at 0.6 cuts unsupported citations a further \
                    39% on the test split for a 1.6-point answer-rate cost.",
                "evidence": "E002 on the held-out test split; threshold chosen on dev (see sweep \
                    in the conversation).",
                "improvement": "Combined with reranking: 17.6% → 6.9% unsupported citations end \
                    to end.",
                "remaining_debt": "Multi-hop answers now fall back to “I couldn’t verify this” \
                    more often.",
                "next_move": "Monitor production answer rate for a week before widening the \
                    rollout.",
                "impact": "CRITICAL",
            }),
        )
        .await?;
        let hypothesis = self.artifact("H002").await?;
        self.knowledge(
            "hypotheses/H002/decision",
            json!({
                "status": "CONFIRMED",
                "conclusion": "Supported by E002 on the test split.",
                "expected_version": version(&hypothesis),
            }),
        )
        .await?;
        self.say(
            "Both hypotheses hold. End to end, unsupported citations went from **17.6% → 6.9%** \
             with a 1.9-point answer-rate cost ([[F001]], [[F002]]). I’ll wait for your go-ahead \
             before proposing a rollout plan.",
        )
        .await?;
        Ok(confirmed_checkpoint(
            "Confirmed H002 on the test split and published F002.",
            "Propose a staged rollout once you approve the combined configuration",
        ))
    }

    async fn follow_up(&mut self, guidance: bool) -> Result<RuntimeDecision> {
        if guidance {
            self.say("Picking up your latest guidance.").await?;
        }
        self.run("python scripts/report.py --since last-checkpoint", 1.5, 0).await?;
        self.say(
            "Summary of where things stand:\n\n\
             - Reranking ([[F001]]) and the grounding check ([[F002]]) are both confirmed.\n\
             - The combined configuration is ready behind the `citation_v2` flag.\n\n\
             Send me any follow-up question or a new direction and I’ll pick it up in the next \
             turn.",
        )
        .await?;
        Ok(confirmed_checkpoint(
            "Reported status and waited for direction.",
            "Wait for the next direction",
        ))
    }
}

fn confirmed_checkpoint(summary: &str, next: &str) -> RuntimeDecision {
    let mut checkpoint = decision(summary, "WAITING", OBJECTIVE, next, "None");
    checkpoint.mission_update = Some(Box::new(MissionUpdate {
        headline: "The grounding check holds on test. Production validation remains.".into(),
        established: "On held-out test, threshold 0.6 gives 6.9% unsupported citations versus 11.4% with reranking alone, at 89.4% answer rate. These are illustrative demo measurements.".into(),
        ruled_out: "More aggressive thresholds withheld too many answers on dev. The agreed two-point answer-rate guardrail was retained.".into(),
        uncertain: "Production behavior and sustained answer rate have not been observed. Offline test results do not establish a safe rollout.".into(),
        evidence: vec!["F001".into(), "E002".into(), "F002".into()], decision: None,
    }));
    checkpoint
}
