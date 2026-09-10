//! Opt-in live smoke test against a real provider. Costs real money and needs
//! network access, so it runs only when explicitly requested:
//!
//! ```bash
//! BUTTERFLY_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p butterfly-core --test live_deepseek -- --nocapture
//! # any registry model: BUTTERFLY_LIVE_MODEL=glm-5.3-flash ZAI_API_KEY=... (same command)
//! ```

use std::sync::Arc;

use butterfly_core::model::Registry;
use butterfly_core::provider::openai_compat::OpenAiCompatProvider;
use butterfly_core::provider::{NullSink, Provider, ProviderRequest};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn live_streaming_round_trip_reports_cache_counters() {
    if std::env::var("BUTTERFLY_LIVE_TEST").as_deref() != Ok("1") {
        eprintln!("skipped: set BUTTERFLY_LIVE_TEST=1 to run");
        return;
    }
    let registry = Registry::builtin();
    let model_id =
        std::env::var("BUTTERFLY_LIVE_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let model = registry.resolve(&model_id).expect("registry model");
    let descriptor = registry.provider_for(&model).expect("provider");
    let api_key = std::env::var(&descriptor.api_key_env)
        .unwrap_or_else(|_| panic!("{} is required", descriptor.api_key_env));
    let provider: Arc<dyn Provider> = Arc::new(
        OpenAiCompatProvider::from_descriptor(descriptor, api_key, None).expect("endpoint"),
    );
    let request = ProviderRequest {
        effort: model.resolve_effort(None).unwrap(),
        model,
        messages: vec![
            butterfly_core::domain::ChatMessage::System {
                content: "You are a terse assistant.".into(),
            },
            butterfly_core::domain::ChatMessage::User {
                content: "Reply with the single word: ok".into(),
            },
        ],
        tools: butterfly_core::tools::tool_schemas(),
        max_output_tokens: 512,
        routing: None,
    };
    let cancel = CancellationToken::new();

    let first = provider
        .stream(&request, &cancel, &mut NullSink)
        .await
        .expect("first request");
    assert!(first.usage.input_tokens > 0);
    assert!(first.usage.output_tokens > 0);
    eprintln!(
        "first: in={} hit={} miss={} out={} cost={}µ$ ({}ms, {} retries)",
        first.usage.input_tokens,
        first.usage.cache_hit_input_tokens,
        first.usage.cache_miss_input_tokens,
        first.usage.output_tokens,
        first.cost.actual_microusd,
        first.latency_ms,
        first.retries
    );

    // Identical prefix + one appended message: the second request should see
    // non-zero cache hits if automatic prefix caching is live.
    let mut second_request = request.clone();
    second_request
        .messages
        .push(butterfly_core::domain::ChatMessage::Assistant(
            first.message.clone(),
        ));
    second_request
        .messages
        .push(butterfly_core::domain::ChatMessage::User {
            content: "Again, reply with: ok".into(),
        });
    let second = provider
        .stream(&second_request, &cancel, &mut NullSink)
        .await
        .expect("second request");
    eprintln!(
        "second: in={} hit={} miss={} out={}",
        second.usage.input_tokens,
        second.usage.cache_hit_input_tokens,
        second.usage.cache_miss_input_tokens,
        second.usage.output_tokens
    );
    // OpenAI caches only prefixes of 1,024+ tokens; smaller probes cannot
    // prove caching, so the assertion applies when the prefix is big enough.
    if first.usage.input_tokens >= 1_024 {
        assert!(
            second.usage.cache_hit_input_tokens > 0,
            "expected prefix-cache hits on the second request"
        );
    } else {
        eprintln!(
            "note: prefix is {} tokens (<1024); cache assertion skipped for this provider",
            first.usage.input_tokens
        );
    }
}
