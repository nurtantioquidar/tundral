//! Opt-in live compaction exercise: drives a real multi-turn agent session
//! with a deliberately tiny context window so stage-1 elision triggers
//! naturally, then forces stage-2 summarization, then verifies the rebuilt
//! history is accepted by the real API and the session continues coherently.
//!
//! ```bash
//! BUTTERFLY_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p butterfly-core --test live_compaction -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use butterfly_core::context::ContextConfig;
use butterfly_core::domain::{format_microusd_as_usd, ChatMessage};
use butterfly_core::model::Registry;
use butterfly_core::provider::openai_compat::OpenAiCompatProvider;
use butterfly_core::provider::NullSink;
use butterfly_core::session::approval::{AutoApprove, PermissionMode};
use butterfly_core::session::{Agent, SessionConfig};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn live_two_stage_compaction_and_continuation() {
    if std::env::var("BUTTERFLY_LIVE_TEST").as_deref() != Ok("1") {
        eprintln!("skipped: set BUTTERFLY_LIVE_TEST=1 to run");
        return;
    }
    let registry = Registry::builtin();
    let model = registry
        .resolve("deepseek-v4-flash")
        .expect("builtin model");
    let descriptor = registry.provider_for(&model).expect("provider");
    let api_key = std::env::var(&descriptor.api_key_env).expect("DEEPSEEK_API_KEY");
    let provider = Arc::new(
        OpenAiCompatProvider::from_descriptor(descriptor, api_key, None).expect("endpoint"),
    );

    // The butterfly workspace itself is the repository under test.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();

    // Small window so a few file reads cross the compact threshold — but not
    // so small that compaction cannot win. The rebuilt history has a floor of
    // system prompt + tool schemas (~3.5k) + checkpoint (max_summary_tokens
    // plus the working state, ~1.1k) + the tail budget. At the original 9k
    // window that floor was ~94% of the 6.5k effective window, so summarizing
    // RAISED utilization and the assertion below fired on correct behaviour.
    // 16k leaves 13.5k effective: the floor is ~7.3k (54%), comfortably under
    // the 0.80 compact threshold, while two turns of reads still cross it.
    let config = SessionConfig {
        effort: model.resolve_effort(None).unwrap(),
        model,
        permission_mode: PermissionMode::ReadOnly,
        max_output_tokens: 4_096,
        max_session_cost_microusd: Some(100_000), // $0.10 hard stop
        max_provider_calls_per_input: 10,
        context: ContextConfig {
            window_tokens: 16_000,
            output_reserve_tokens: 2_000,
            safety_margin_tokens: 500,
            min_savings_fraction: 0.05,
            protected_tail_tokens: 1_500,
            keep_last_turns: 1,
            // Tighter than the 0.35 default: the tail is the one term of the
            // rebuilt floor that scales with the window, so capping it is what
            // keeps compaction able to reduce utilization at this size.
            tail_budget_fraction: 0.20,
            max_summary_tokens: 700,
            ..ContextConfig::default()
        },
        atlas_enabled: true,
        sandboxed: false,
        task_type: None,
        completion_gate: false,
        lean: false,
        clarify: false,
        independent_verifier: false,
        verifier: false,
        outline_min_lines: 400,
        always_review_claim: false,
    };
    let mut agent = Agent::new(provider, root, config, None).expect("agent");
    let cancel = CancellationToken::new();

    let turns = [
        "Use the read_file tool to read README.md and AGENTS.md, then answer with one sentence per file.",
        "Use the read_file tool to read crates/core/src/context/prompt.rs and crates/core/src/provider/sse.rs, then answer with one sentence per file.",
        "Use the read_file tool to read crates/core/src/session/approval.rs, then answer with one sentence about it.",
    ];
    let mut elisions = 0u32;
    let mut peak_utilization = 0.0f64;
    for (index, input) in turns.iter().enumerate() {
        let report = agent
            .run_turn(input.to_string(), &AutoApprove, &mut NullSink, &cancel)
            .await
            .expect("turn");
        elisions += report.elisions;
        peak_utilization = peak_utilization.max(agent.utilization());
        eprintln!(
            "turn {}: calls={} tools={} in={} (hit {:.0}%) out={} elisions={} summarize={} util={:.0}% cost={}",
            index + 1,
            report.provider_calls,
            report.tool_calls,
            report.usage.input_tokens,
            report.usage.cache_hit_rate() * 100.0,
            report.usage.output_tokens,
            report.elisions,
            report.compactions,
            agent.utilization() * 100.0,
            format_microusd_as_usd(report.actual_microusd),
        );
        assert!(
            !report.budget_exhausted,
            "budget blew on turn {}",
            index + 1
        );
        assert!(
            !report.assistant_text.is_empty(),
            "empty answer on turn {}",
            index + 1
        );
        if report.elisions > 0 && report.compactions == 0 {
            // Placeholders must be live in the accepted history right now.
            // Gated on no summarization in THIS turn: stage 2 rebuilds the
            // history from the checkpoint plus a short tail, which correctly
            // drops placeholders along with the turns that held them. Under a
            // window this small a single large read can push one turn through
            // both stages, and the placeholder is then legitimately gone.
            assert!(
                agent.messages().iter().any(
                    |m| matches!(m, ChatMessage::Tool { content, .. } if content.starts_with("[elided"))
                ),
                "turn {}: elided {} result(s) but no placeholder survived",
                index + 1,
                report.elisions
            );
        }
    }
    assert!(
        elisions >= 1,
        "expected stage-1 elision to trigger under the tiny window"
    );
    // Compaction across the turn sequence must bring the window down: this is
    // the reduction that matters, measured where the loop actually invokes it.
    assert!(
        agent.utilization() < peak_utilization,
        "turn-driven compaction never reduced the window: peak {:.0}%, now {:.0}%",
        peak_utilization * 100.0,
        agent.utilization() * 100.0
    );

    // Stage 2, forced (the /compact path), against the live API.
    let before_utilization = agent.utilization();
    agent
        .compact_now(&cancel, None)
        .await
        .expect("live summarize");
    eprintln!(
        "compact: util {:.0}% -> {:.0}%",
        before_utilization * 100.0,
        agent.utilization() * 100.0
    );
    match &agent.messages()[1] {
        ChatMessage::User { content } => {
            assert!(
                content.starts_with("CONTEXT CHECKPOINT"),
                "checkpoint shape"
            );
            assert!(
                content.contains("## Working state"),
                "working-state section survived"
            );
            eprintln!("checkpoint head:\n{}", &content[..content.len().min(600)]);
        }
        other => panic!("expected checkpoint at messages[1], got {other:?}"),
    }
    let checkpoints = agent
        .messages()
        .iter()
        .filter(
            |m| matches!(m, ChatMessage::User { content } if content.starts_with("CONTEXT CHECKPOINT")),
        )
        .count();
    assert_eq!(checkpoints, 1, "checkpoints must supersede, never stack");
    // NOT "compaction always lowers utilization" — that is false, and it is
    // arithmetic rather than a bug. The rebuilt history has a floor of system
    // prompt + checkpoint + tail budget; forced from below that floor, as
    // /compact on an already-compacted session is, a rebuild legitimately
    // raises utilization. What `tail_budget_fraction` guarantees, and what
    // keeps the loop from thrashing, is that the rebuild lands under the
    // compact threshold so the next turn does not immediately summarize again.
    let compact_threshold = ContextConfig::default().compact_threshold;
    assert!(
        agent.utilization() < compact_threshold,
        "rebuild must land below the compact threshold: {:.0}% >= {:.0}%",
        agent.utilization() * 100.0,
        compact_threshold * 100.0
    );
    if before_utilization >= compact_threshold {
        assert!(
            agent.utilization() < before_utilization,
            "compaction above the threshold must reduce the window: {:.0}% -> {:.0}%",
            before_utilization * 100.0,
            agent.utilization() * 100.0
        );
    }

    // Continuation on the rebuilt history must be accepted and coherent.
    let report = agent
        .run_turn(
            "Based on this session so far, list the file paths you have read.".to_string(),
            &AutoApprove,
            &mut NullSink,
            &cancel,
        )
        .await
        .expect("post-compaction turn");
    eprintln!(
        "post-compact turn: in={} (hit {:.0}%) out={}\nanswer:\n{}",
        report.usage.input_tokens,
        report.usage.cache_hit_rate() * 100.0,
        report.usage.output_tokens,
        report.assistant_text
    );
    assert!(!report.assistant_text.is_empty());
    assert!(
        report.assistant_text.contains("approval"),
        "continuation lost the session state: {}",
        report.assistant_text
    );
    eprintln!(
        "session total: {} (cap $0.10)",
        format_microusd_as_usd(agent.total_actual_microusd())
    );
}
