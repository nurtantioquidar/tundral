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
use butterfly_core::domain::{format_microusd_as_usd, ChatMessage, Model};
use butterfly_core::provider::deepseek::DeepSeekProvider;
use butterfly_core::provider::{NullSink, ReasoningEffort};
use butterfly_core::session::approval::{AutoApprove, PermissionMode};
use butterfly_core::session::{Agent, SessionConfig};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn live_two_stage_compaction_and_continuation() {
    if std::env::var("BUTTERFLY_LIVE_TEST").as_deref() != Ok("1") {
        eprintln!("skipped: set BUTTERFLY_LIVE_TEST=1 to run");
        return;
    }
    let api_key = std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY");
    let base_url = std::env::var("DEEPSEEK_BASE_URL").ok();
    let provider = Arc::new(DeepSeekProvider::new(api_key, base_url));

    // The butterfly workspace itself is the repository under test.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();

    // Tiny window so a few file reads cross the compact threshold.
    let config = SessionConfig {
        model: Model::DeepseekV4Flash,
        effort: ReasoningEffort::High,
        permission_mode: PermissionMode::ReadOnly,
        max_output_tokens: 4_096,
        max_session_cost_microusd: Some(100_000), // $0.10 hard stop
        max_provider_calls_per_input: 10,
        context: ContextConfig {
            window_tokens: 9_000,
            output_reserve_tokens: 2_000,
            safety_margin_tokens: 500,
            min_savings_fraction: 0.05,
            protected_tail_tokens: 1_500,
            keep_last_turns: 1,
            max_summary_tokens: 700,
            ..ContextConfig::default()
        },
        atlas_enabled: true,
    };
    let mut agent = Agent::new(provider, root, config, None).expect("agent");
    let cancel = CancellationToken::new();

    let turns = [
        "Use the read_file tool to read README.md and AGENTS.md, then answer with one sentence per file.",
        "Use the read_file tool to read crates/core/src/context/prompt.rs and crates/core/src/provider/sse.rs, then answer with one sentence per file.",
        "Use the read_file tool to read crates/core/src/session/approval.rs, then answer with one sentence about it.",
    ];
    let mut elisions = 0u32;
    for (index, input) in turns.iter().enumerate() {
        let report = agent
            .run_turn(input.to_string(), &AutoApprove, &mut NullSink, &cancel)
            .await
            .expect("turn");
        elisions += report.elisions;
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
        if report.elisions > 0 {
            // Placeholders must be live in the accepted history right now (a
            // LATER summarize may legitimately drop them with their turns).
            assert!(agent.messages().iter().any(
                |m| matches!(m, ChatMessage::Tool { content, .. } if content.starts_with("[elided"))
            ));
        }
    }
    assert!(
        elisions >= 1,
        "expected stage-1 elision to trigger under the tiny window"
    );

    // Stage 2, forced (the /compact path), against the live API.
    let before_utilization = agent.utilization();
    agent.compact_now(&cancel).await.expect("live summarize");
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
                content.contains("## Mechanical state"),
                "mechanical section survived"
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
    assert!(agent.utilization() < before_utilization);

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
