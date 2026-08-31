//! Opt-in live smoke test. Costs real money and needs network access, so it
//! runs only when explicitly requested:
//!
//! ```bash
//! BUTTERFLY_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p butterfly-core --test live_deepseek -- --nocapture
//! ```

use std::sync::Arc;

use butterfly_core::domain::Model;
use butterfly_core::provider::deepseek::DeepSeekProvider;
use butterfly_core::provider::{NullSink, Provider, ProviderRequest, ReasoningEffort};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn live_streaming_round_trip_reports_cache_counters() {
    if std::env::var("BUTTERFLY_LIVE_TEST").as_deref() != Ok("1") {
        eprintln!("skipped: set BUTTERFLY_LIVE_TEST=1 to run");
        return;
    }
    let api_key = std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY");
    let base_url = std::env::var("DEEPSEEK_BASE_URL").ok();
    let provider: Arc<dyn Provider> = Arc::new(DeepSeekProvider::new(api_key, base_url));
    let request = ProviderRequest {
        model: Model::DeepseekV4Flash,
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
        effort: ReasoningEffort::High,
    };
    let cancel = CancellationToken::new();

    let first = provider
        .stream(&request, &cancel, &mut NullSink)
        .await
        .expect("first request");
    assert!(first.usage.input_tokens > 0);
    assert!(first.usage.output_tokens > 0);
    eprintln!(
        "first: in={} hit={} miss={} out={} cost={}µ$",
        first.usage.input_tokens,
        first.usage.cache_hit_input_tokens,
        first.usage.cache_miss_input_tokens,
        first.usage.output_tokens,
        first.cost.actual_microusd
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
    assert!(
        second.usage.cache_hit_input_tokens > 0,
        "expected prefix-cache hits on the second request"
    );
}
