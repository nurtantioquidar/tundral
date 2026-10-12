use clap::ValueEnum;
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::env;
use std::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MICROS_PER_UNIT: i128 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Openai,
    #[value(alias = "muse", alias = "meta-ai", alias = "meta-model-api")]
    Meta,
    #[value(alias = "z-ai", alias = "glm")]
    Zai,
    Deepseek,
}

impl Provider {
    pub const ALL: [Self; 4] = [Self::Openai, Self::Meta, Self::Zai, Self::Deepseek];
}

impl fmt::Display for Provider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Openai => "OpenAI",
            Self::Meta => "Meta (Muse)",
            Self::Zai => "Z.ai",
            Self::Deepseek => "DeepSeek",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Unconfigured,
    Unavailable,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Balance {
    pub currency: String,
    pub total: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topped_up: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Quota {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_percent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<String>,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckResult {
    pub provider: Provider,
    pub status: Status,
    pub balances: Vec<Balance>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quotas: Vec<Quota>,
    pub detail: String,
    pub elapsed_ms: u128,
}

impl CheckResult {
    fn new(provider: Provider, status: Status, detail: impl Into<String>) -> Self {
        Self {
            provider,
            status,
            balances: Vec::new(),
            quotas: Vec::new(),
            detail: detail.into(),
            elapsed_ms: 0,
        }
    }

    fn ok(provider: Provider, balances: Vec<Balance>, detail: impl Into<String>) -> Self {
        Self {
            provider,
            status: Status::Ok,
            balances,
            quotas: Vec::new(),
            detail: detail.into(),
            elapsed_ms: 0,
        }
    }

    fn ok_with(
        provider: Provider,
        balances: Vec<Balance>,
        quotas: Vec<Quota>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            status: Status::Ok,
            balances,
            quotas,
            detail: detail.into(),
            elapsed_ms: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DecimalAmount(i128);

impl DecimalAmount {
    fn parse(raw: &str) -> Result<Self, String> {
        let cleaned = raw.trim().trim_start_matches('$').replace(',', "");
        if cleaned.is_empty() {
            return Err("empty amount".into());
        }

        let (negative, unsigned) = match cleaned.as_bytes()[0] {
            b'-' => (true, &cleaned[1..]),
            b'+' => (false, &cleaned[1..]),
            _ => (false, cleaned.as_str()),
        };
        let mut parts = unsigned.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next().unwrap_or_default();
        if parts.next().is_some()
            || (whole.is_empty() && fraction.is_empty())
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(format!("invalid decimal amount `{raw}`"));
        }

        let whole_value = if whole.is_empty() {
            0
        } else {
            whole
                .parse::<i128>()
                .map_err(|_| format!("amount `{raw}` is too large"))?
        };
        let mut fractional_value = 0_i128;
        for index in 0..6 {
            fractional_value *= 10;
            if let Some(byte) = fraction.as_bytes().get(index) {
                fractional_value += i128::from(byte - b'0');
            }
        }
        if fraction
            .as_bytes()
            .get(6)
            .is_some_and(|digit| *digit >= b'5')
        {
            fractional_value += 1;
        }

        let micros = whole_value
            .checked_mul(MICROS_PER_UNIT)
            .and_then(|value| value.checked_add(fractional_value))
            .ok_or_else(|| format!("amount `{raw}` is too large"))?;
        Ok(Self(if negative { -micros } else { micros }))
    }

    fn checked_add(self, other: Self) -> Result<Self, String> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or_else(|| "amount total overflowed".into())
    }

    fn checked_sub(self, other: Self) -> Result<Self, String> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or_else(|| "amount difference overflowed".into())
    }

    fn render(self) -> String {
        let negative = self.0 < 0;
        let absolute = self.0.abs();
        let whole = absolute / MICROS_PER_UNIT;
        let fraction = absolute % MICROS_PER_UNIT;
        let sign = if negative { "-" } else { "" };
        if fraction == 0 {
            format!("{sign}{whole}")
        } else {
            let fraction = format!("{fraction:06}").trim_end_matches('0').to_string();
            format!("{sign}{whole}.{fraction}")
        }
    }
}

#[derive(Clone)]
pub struct BalanceChecker {
    client: Client,
}

impl BalanceChecker {
    pub fn new(timeout: Duration) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("cicada/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("could not create HTTP client: {error}"))?;
        Ok(Self { client })
    }

    pub async fn check(&self, provider: Provider) -> CheckResult {
        let started = Instant::now();
        let mut result = match provider {
            Provider::Deepseek => self.check_deepseek().await,
            Provider::Zai => self.check_zai().await,
            Provider::Meta => self.check_meta().await,
            Provider::Openai => self.check_openai().await,
        };
        result.elapsed_ms = started.elapsed().as_millis();
        result
    }

    async fn check_deepseek(&self) -> CheckResult {
        let provider = Provider::Deepseek;
        let Some(key) = first_env(&["DEEPSEEK_API_KEY"]) else {
            return CheckResult::new(provider, Status::Unconfigured, "set DEEPSEEK_API_KEY");
        };
        let url = env::var("DEEPSEEK_BALANCE_URL").unwrap_or_else(|_| {
            endpoint(
                &env::var("DEEPSEEK_BASE_URL")
                    .unwrap_or_else(|_| "https://api.deepseek.com".into()),
                "user/balance",
            )
        });
        match self.get_json(&url, &key).await {
            Ok(value) => match parse_deepseek_balance(&value) {
                Ok(balances) => CheckResult::ok(provider, balances, "exact provider balance"),
                Err(error) => response_shape_error(provider, error),
            },
            Err(error) => request_error(provider, error, true),
        }
    }

    async fn check_zai(&self) -> CheckResult {
        let provider = Provider::Zai;
        let key_names = &["ZAI_API_KEY"];
        let Some(key) = first_env(key_names) else {
            return CheckResult::new(
                provider,
                Status::Unconfigured,
                format!("set one of {}", key_names.join(", ")),
            );
        };
        let configured_base = env::var("ZAI_MONITOR_BASE_URL")
            .or_else(|_| env::var("ZAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.z.ai".into());
        let monitor_base = zai_origin(&configured_base);
        let balance_url = env::var("ZAI_BALANCE_URL")
            .or_else(|_| env::var("ZAI_CREDITS_URL"))
            .unwrap_or_else(|_| {
                endpoint(
                    &monitor_base,
                    "api/biz/account/query-customer-account-report",
                )
            });
        let quota_url = env::var("ZAI_QUOTA_URL")
            .unwrap_or_else(|_| endpoint(&monitor_base, "api/monitor/usage/quota/limit"));

        let (balance_response, quota_response) = tokio::join!(
            self.get_json(&balance_url, &key),
            self.get_json(&quota_url, &key)
        );
        let balances = balance_response
            .as_ref()
            .ok()
            .and_then(|value| parse_zai_balance(value).ok())
            .unwrap_or_default();
        let quotas = quota_response
            .as_ref()
            .ok()
            .and_then(|value| parse_zai_quotas(value).ok())
            .unwrap_or_default();

        if !balances.is_empty() || !quotas.is_empty() {
            let detail = match (!balances.is_empty(), !quotas.is_empty()) {
                (true, true) => "account balance and coding-plan quota",
                (true, false) => "account balance; no coding plan",
                (false, true) => "coding-plan quota; account balance unavailable",
                (false, false) => unreachable!(),
            };
            return CheckResult::ok_with(provider, balances, quotas, detail);
        }

        let balance_error = endpoint_result_error("balance", &balance_response, parse_zai_balance);
        let quota_error = endpoint_result_error("quota", &quota_response, parse_zai_quotas);
        let status = if balance_error.not_found && quota_error.not_found {
            Status::Unavailable
        } else {
            Status::Error
        };
        CheckResult::new(
            provider,
            status,
            format!("{}; {}", balance_error.message, quota_error.message),
        )
    }

    async fn check_meta(&self) -> CheckResult {
        let provider = Provider::Meta;
        let key_names = &["META_MODEL_API_KEY", "META_API_KEY", "MODEL_API_KEY"];
        let Some(key) = first_env(key_names) else {
            return CheckResult::new(
                provider,
                Status::Unconfigured,
                format!("set one of {}", key_names.join(", ")),
            );
        };
        let url = env::var("META_BALANCE_URL").unwrap_or_else(|_| {
            endpoint(
                &env::var("META_BASE_URL").unwrap_or_else(|_| "https://api.meta.ai/v1".into()),
                "balance",
            )
        });
        match self.get_json(&url, &key).await {
            Ok(value) => match parse_generic_balance(&value, "USD") {
                Ok(balances) => CheckResult::ok(
                    provider,
                    balances,
                    "provider balance route (not in the public API specification)",
                ),
                Err(error) => response_shape_error(provider, error),
            },
            Err(error) => request_error(provider, error, false),
        }
    }

    async fn check_openai(&self) -> CheckResult {
        let provider = Provider::Openai;
        let Some(key) = first_env(&["OPENAI_ADMIN_KEY"]) else {
            let detail = if env::var_os("OPENAI_API_KEY").is_some() {
                "OPENAI_API_KEY cannot read organization costs; set OPENAI_ADMIN_KEY and OPENAI_MONTHLY_BUDGET_USD"
            } else {
                "set OPENAI_ADMIN_KEY and OPENAI_MONTHLY_BUDGET_USD"
            };
            return CheckResult::new(provider, Status::Unconfigured, detail);
        };
        let budget_raw =
            match env::var("OPENAI_MONTHLY_BUDGET_USD") {
                Ok(value) if !value.trim().is_empty() => value,
                _ => return CheckResult::new(
                    provider,
                    Status::Unconfigured,
                    "set OPENAI_MONTHLY_BUDGET_USD; OpenAI has no public prepaid-balance endpoint",
                ),
            };
        let budget = match DecimalAmount::parse(&budget_raw) {
            Ok(value) if value.0 >= 0 => value,
            Ok(_) => {
                return CheckResult::new(
                    provider,
                    Status::Unconfigured,
                    "OPENAI_MONTHLY_BUDGET_USD cannot be negative",
                )
            }
            Err(error) => return CheckResult::new(provider, Status::Unconfigured, error),
        };
        let start_time = match start_of_current_month_utc() {
            Ok(value) => value,
            Err(error) => return CheckResult::new(provider, Status::Error, error),
        };
        let base =
            env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let url = format!(
            "{}?start_time={start_time}&bucket_width=1d&limit=31",
            endpoint(&base, "organization/costs")
        );
        match self.get_json(&url, &key).await {
            Ok(value) => match parse_openai_costs(&value) {
                Ok(spent) => match budget.checked_sub(spent) {
                    Ok(remaining) => CheckResult::ok(
                        provider,
                        vec![Balance {
                            currency: "USD".into(),
                            total: remaining.render(),
                            granted: None,
                            topped_up: None,
                        }],
                        format!(
                            "computed monthly remainder: {} budget - {} month-to-date spend",
                            budget.render(),
                            spent.render()
                        ),
                    ),
                    Err(error) => CheckResult::new(provider, Status::Error, error),
                },
                Err(error) => response_shape_error(provider, error),
            },
            Err(error) => request_error(provider, error, true),
        }
    }

    async fn get_json(&self, url: &str, key: &str) -> Result<Value, HttpFailure> {
        let response = self
            .client
            .get(url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|error| HttpFailure::Transport(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| HttpFailure::Transport(error.to_string()))?;
        let value = serde_json::from_slice::<Value>(&bytes).ok();
        if !status.is_success() {
            let message = value
                .as_ref()
                .and_then(provider_error_message)
                .unwrap_or_else(|| status.canonical_reason().unwrap_or("request failed").into());
            return Err(HttpFailure::Status(status, message));
        }
        value.ok_or_else(|| HttpFailure::InvalidJson("provider returned non-JSON data".into()))
    }
}

#[derive(Debug)]
enum HttpFailure {
    Status(StatusCode, String),
    Transport(String),
    InvalidJson(String),
}

struct EndpointIssue {
    not_found: bool,
    message: String,
}

fn endpoint_result_error<T>(
    label: &str,
    result: &Result<Value, HttpFailure>,
    parser: impl FnOnce(&Value) -> Result<Vec<T>, String>,
) -> EndpointIssue {
    match result {
        Ok(value) => EndpointIssue {
            not_found: false,
            message: match parser(value) {
                Ok(_) => format!("{label} endpoint returned no usable values"),
                Err(error) => format!("{label}: {error}"),
            },
        },
        Err(HttpFailure::Status(status, message)) => EndpointIssue {
            not_found: *status == StatusCode::NOT_FOUND,
            message: format!(
                "{label}: HTTP {}: {}",
                status.as_u16(),
                truncate(message, 120)
            ),
        },
        Err(HttpFailure::Transport(message) | HttpFailure::InvalidJson(message)) => EndpointIssue {
            not_found: false,
            message: format!("{label}: {}", truncate(message, 120)),
        },
    }
}

fn request_error(provider: Provider, error: HttpFailure, documented: bool) -> CheckResult {
    match error {
        HttpFailure::Status(status, _) if status == StatusCode::NOT_FOUND && !documented => {
            CheckResult::new(
                provider,
                Status::Unavailable,
                "this account/API region does not expose a balance route",
            )
        }
        HttpFailure::Status(status, message) => CheckResult::new(
            provider,
            Status::Error,
            format!("HTTP {}: {}", status.as_u16(), truncate(&message, 180)),
        ),
        HttpFailure::Transport(message) | HttpFailure::InvalidJson(message) => {
            CheckResult::new(provider, Status::Error, truncate(&message, 180))
        }
    }
}

fn response_shape_error(provider: Provider, error: String) -> CheckResult {
    let status = match provider {
        Provider::Meta | Provider::Zai => Status::Unavailable,
        Provider::Openai | Provider::Deepseek => Status::Error,
    };
    CheckResult::new(provider, status, error)
}

fn first_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    })
}

fn endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn zai_origin(base: &str) -> String {
    let base = base.trim_end_matches('/');
    for suffix in ["/api/coding/paas/v4", "/api/paas/v4", "/api/anthropic"] {
        if let Some(origin) = base.strip_suffix(suffix) {
            return origin.to_string();
        }
    }
    base.to_string()
}

fn provider_error_message(value: &Value) -> Option<String> {
    ["/error/message", "/message", "/msg"]
        .into_iter()
        .find_map(|pointer| value.pointer(pointer).and_then(value_as_text))
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn parse_deepseek_balance(value: &Value) -> Result<Vec<Balance>, String> {
    let infos = value
        .get("balance_infos")
        .and_then(Value::as_array)
        .ok_or_else(|| "DeepSeek response did not contain `balance_infos`".to_string())?;
    if infos.is_empty() {
        return Err("DeepSeek returned no balances".into());
    }
    infos
        .iter()
        .map(|info| {
            let currency = info
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or("USD")
                .to_uppercase();
            Ok(Balance {
                currency,
                total: parse_json_amount(
                    info.get("total_balance")
                        .ok_or_else(|| "DeepSeek balance missed `total_balance`".to_string())?,
                )?
                .render(),
                granted: optional_json_amount(info.get("granted_balance"))?,
                topped_up: optional_json_amount(info.get("topped_up_balance"))?,
            })
        })
        .collect()
}

fn parse_generic_balance(value: &Value, default_currency: &str) -> Result<Vec<Balance>, String> {
    const POINTERS: &[(&str, &str)] = &[
        ("/data/total_available", "total_available"),
        ("/data/totalAvailable", "total_available"),
        ("/data/available_amount", "available_amount"),
        ("/data/credits_available", "credits_available"),
        ("/data/total_balance", "total_balance"),
        ("/data/remaining_balance", "remaining_balance"),
        ("/data/available_balance", "available_balance"),
        ("/data/balance", "balance"),
        ("/total_available", "total_available"),
        ("/totalAvailable", "total_available"),
        ("/available_amount", "available_amount"),
        ("/credits_available", "credits_available"),
        ("/total_balance", "total_balance"),
        ("/remaining_balance", "remaining_balance"),
        ("/available_balance", "available_balance"),
        ("/balance", "balance"),
        ("/data/credits", "credits"),
        ("/credits", "credits"),
        ("/data/points", "points"),
        ("/points", "points"),
    ];

    for (pointer, name) in POINTERS {
        if let Some(candidate) = value.pointer(pointer) {
            let amount_value = if let Value::Object(object) = candidate {
                [
                    "total",
                    "remaining",
                    "available",
                    "available_amount",
                    "amount",
                    "value",
                ]
                .into_iter()
                .find_map(|key| object.get(key))
                .unwrap_or(candidate)
            } else {
                candidate
            };
            if let Ok(amount) = parse_json_amount(amount_value) {
                let currency = if *name == "points" {
                    "POINTS".into()
                } else {
                    find_text_by_key(value, &["currency", "currency_code"])
                        .unwrap_or_else(|| default_currency.into())
                        .to_uppercase()
                };
                return Ok(vec![Balance {
                    currency,
                    total: amount.render(),
                    granted: None,
                    topped_up: None,
                }]);
            }
        }
    }

    Err("provider response did not contain a recognized balance field".into())
}

fn parse_zai_balance(value: &Value) -> Result<Vec<Balance>, String> {
    ensure_zai_success(value)?;
    if let Ok(balance) = parse_generic_balance(value, "USD") {
        return Ok(balance);
    }

    let grants = [
        "/data/credit_grants/grants",
        "/data/creditGrants/grants",
        "/credit_grants/grants",
        "/creditGrants/grants",
        "/data/grants",
        "/grants",
        "/data",
    ]
    .into_iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_array))
    .ok_or_else(|| "Z.ai credit response contained no grants".to_string())?;
    let mut total = DecimalAmount(0);
    let mut found = false;
    for grant in grants {
        let remaining = [
            "remaining",
            "available",
            "available_amount",
            "availableAmount",
            "remaining_amount",
            "remainingAmount",
            "credits_available",
        ]
        .into_iter()
        .find_map(|field| grant.get(field))
        .map(parse_json_amount)
        .transpose()?;
        let remaining = match remaining {
            Some(amount) => Some(amount),
            None => {
                let amount = ["grant_amount", "grantAmount", "amount"]
                    .into_iter()
                    .find_map(|field| grant.get(field))
                    .map(parse_json_amount)
                    .transpose()?;
                let used = ["used_amount", "usedAmount", "used"]
                    .into_iter()
                    .find_map(|field| grant.get(field))
                    .map(parse_json_amount)
                    .transpose()?
                    .unwrap_or(DecimalAmount(0));
                amount.map(|amount| amount.checked_sub(used)).transpose()?
            }
        };
        if let Some(remaining) = remaining {
            total = total.checked_add(remaining)?;
            found = true;
        }
    }
    if !found {
        return Err("Z.ai grants contained no recognized amounts".into());
    }
    let currency = find_text_by_key(value, &["currency", "currency_code"])
        .unwrap_or_else(|| "USD".into())
        .to_uppercase();
    Ok(vec![Balance {
        currency,
        total: total.render(),
        granted: Some(total.render()),
        topped_up: None,
    }])
}

fn parse_zai_quotas(value: &Value) -> Result<Vec<Quota>, String> {
    ensure_zai_success(value)?;
    let limits = ["/data/limits", "/limits", "/data"]
        .into_iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_array))
        .ok_or_else(|| "Z.ai quota response contained no limits".to_string())?;

    let mut quotas = Vec::new();
    for limit in limits {
        let limit_type = limit
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        if !matches!(limit_type, "TOKENS_LIMIT" | "TIME_LIMIT" | "CREDIT_LIMIT") {
            continue;
        }
        let unit_code = limit.get("unit").and_then(value_as_u64);
        let number = limit.get("number").and_then(value_as_u64);
        let used_percent = limit.get("percentage").map(parse_json_amount).transpose()?;
        let remaining_percent = used_percent
            .map(|used| DecimalAmount(100 * MICROS_PER_UNIT).checked_sub(used))
            .transpose()?
            .map(DecimalAmount::render);
        let remaining = limit
            .get("remaining")
            .map(parse_json_amount)
            .transpose()?
            .map(DecimalAmount::render);
        if remaining_percent.is_none() && remaining.is_none() {
            continue;
        }
        quotas.push(Quota {
            label: zai_quota_label(limit_type, unit_code, number),
            remaining_percent,
            remaining,
            unit: match limit_type {
                "TOKENS_LIMIT" => "tokens",
                "TIME_LIMIT" => "requests",
                "CREDIT_LIMIT" => "credits",
                _ => unreachable!(),
            }
            .into(),
            resets_at_unix_ms: limit.get("nextResetTime").and_then(value_as_u64),
        });
    }
    if quotas.is_empty() {
        Err("Z.ai returned no recognized quota windows".into())
    } else {
        Ok(quotas)
    }
}

fn ensure_zai_success(value: &Value) -> Result<(), String> {
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(provider_error_message(value).unwrap_or_else(|| "Z.ai request failed".into()));
    }
    if let Some(code) = value.get("code").and_then(value_as_text) {
        if code != "0" && code != "200" {
            return Err(provider_error_message(value)
                .unwrap_or_else(|| format!("Z.ai returned business error {code}")));
        }
    }
    Ok(())
}

fn zai_quota_label(limit_type: &str, unit: Option<u64>, number: Option<u64>) -> String {
    let window = match (unit, number) {
        (Some(3), Some(5)) => "5h rolling",
        (Some(6), Some(1)) => "weekly",
        (_, Some(number)) => return format!("{number}-unit {}", zai_quota_kind(limit_type)),
        _ => "account",
    };
    format!("{window} {}", zai_quota_kind(limit_type))
}

fn zai_quota_kind(limit_type: &str) -> &'static str {
    match limit_type {
        "TOKENS_LIMIT" => "tokens",
        "TIME_LIMIT" => "tools",
        "CREDIT_LIMIT" => "credits",
        _ => "quota",
    }
}

fn parse_openai_costs(value: &Value) -> Result<DecimalAmount, String> {
    let buckets = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "OpenAI Costs response did not contain `data`".to_string())?;
    let mut total = DecimalAmount(0);
    for result in buckets.iter().flat_map(|bucket| {
        bucket
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }) {
        let amount = result
            .pointer("/amount/value")
            .ok_or_else(|| "OpenAI cost result missed `amount.value`".to_string())?;
        let currency = result
            .pointer("/amount/currency")
            .and_then(Value::as_str)
            .unwrap_or("usd");
        if !currency.eq_ignore_ascii_case("usd") {
            return Err(format!("OpenAI returned unsupported currency `{currency}`"));
        }
        total = total.checked_add(parse_json_amount(amount)?)?;
    }
    Ok(total)
}

fn parse_json_amount(value: &Value) -> Result<DecimalAmount, String> {
    match value {
        Value::String(text) => DecimalAmount::parse(text),
        Value::Number(number) => DecimalAmount::parse(&number.to_string()),
        _ => Err("balance amount was not a string or number".into()),
    }
}

fn optional_json_amount(value: Option<&Value>) -> Result<Option<String>, String> {
    value
        .map(|value| parse_json_amount(value).map(DecimalAmount::render))
        .transpose()
}

fn value_as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn find_text_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(text) = object.get(*key).and_then(value_as_text) {
                    return Some(text);
                }
            }
            object
                .values()
                .find_map(|child| find_text_by_key(child, keys))
        }
        Value::Array(array) => array.iter().find_map(|child| find_text_by_key(child, keys)),
        _ => None,
    }
}

fn start_of_current_month_utc() -> Result<u64, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_secs();
    Ok(start_of_month_utc_at(seconds))
}

fn start_of_month_utc_at(seconds: u64) -> u64 {
    let days = (seconds / 86_400) as i64;
    let (year, month, _) = civil_from_days(days);
    days_from_civil(year, month, 1) as u64 * 86_400
}

// Howard Hinnant's proleptic-Gregorian civil date algorithms, shifted to the Unix epoch.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let provisional_year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = provisional_year + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decimal_amount_is_integer_micro_units() {
        assert_eq!(
            DecimalAmount::parse("$1,234.56789").unwrap().0,
            1_234_567_890
        );
        assert_eq!(DecimalAmount::parse("0.0000006").unwrap().0, 1);
        assert_eq!(DecimalAmount::parse("-2.50").unwrap().render(), "-2.5");
        assert!(DecimalAmount::parse("1.2.3").is_err());
    }

    #[test]
    fn deepseek_shape_preserves_balance_components() {
        let balances = parse_deepseek_balance(&json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "USD",
                "total_balance": "110.00",
                "granted_balance": "10.00",
                "topped_up_balance": "100.00"
            }]
        }))
        .unwrap();
        assert_eq!(
            balances,
            vec![Balance {
                currency: "USD".into(),
                total: "110".into(),
                granted: Some("10".into()),
                topped_up: Some("100".into()),
            }]
        );
    }

    #[test]
    fn generic_shapes_accept_nested_balance_and_points() {
        assert_eq!(
            parse_generic_balance(
                &json!({"data": {"balance": "12.345", "currency": "usd"}}),
                "USD"
            )
            .unwrap()[0]
                .total,
            "12.345"
        );
        let points = parse_generic_balance(&json!({"points": "42.5"}), "USD").unwrap();
        assert_eq!(points[0].currency, "POINTS");
    }

    #[test]
    fn zai_credit_grants_are_aggregated_without_floats() {
        let balances = parse_zai_balance(&json!({
            "code": 200,
            "success": true,
            "data": [
                {"amount": "10.50", "used_amount": "2.25"},
                {"amount": "3", "used_amount": "0.125"}
            ]
        }))
        .unwrap();
        assert_eq!(balances[0].currency, "USD");
        assert_eq!(balances[0].total, "11.125");
    }

    #[test]
    fn zai_credit_totals_and_nested_grants_match_current_shapes() {
        let total = parse_zai_balance(&json!({
            "total_granted": 100,
            "total_used": 27.5,
            "total_available": 72.5
        }))
        .unwrap();
        assert_eq!(total[0].total, "72.5");

        let nested = parse_zai_balance(&json!({
            "data": {
                "credit_grants": {
                    "grants": [
                        {"grant_amount": 50, "used_amount": 5},
                        {"grantAmount": 20, "usedAmount": 4}
                    ]
                }
            }
        }))
        .unwrap();
        assert_eq!(nested[0].total, "61");
    }

    #[test]
    fn zai_account_report_returns_pay_as_you_go_balance() {
        let balances = parse_zai_balance(&json!({
            "code": 200,
            "success": true,
            "data": {
                "balance": "9.88",
                "availableBalance": "9.88",
                "totalSpendAmount": "4.12"
            }
        }))
        .unwrap();
        assert_eq!(balances[0].currency, "USD");
        assert_eq!(balances[0].total, "9.88");
    }

    #[test]
    fn zai_current_credit_limit_shape_becomes_remaining_quotas() {
        let quotas = parse_zai_quotas(&json!({
            "code": 200,
            "msg": "Operation successful",
            "data": {
                "limits": [
                    {
                        "type": "CREDIT_LIMIT",
                        "unit": 3,
                        "number": 5,
                        "remaining": 0,
                        "percentage": 100,
                        "nextResetTime": 1786624439633_u64
                    },
                    {
                        "type": "CREDIT_LIMIT",
                        "unit": 6,
                        "number": 1,
                        "remaining": 7994,
                        "percentage": 20,
                        "nextResetTime": 1787210351998_u64
                    }
                ]
            },
            "success": true
        }))
        .unwrap();
        assert_eq!(quotas[0].label, "5h rolling credits");
        assert_eq!(quotas[0].remaining_percent.as_deref(), Some("0"));
        assert_eq!(quotas[1].label, "weekly credits");
        assert_eq!(quotas[1].remaining_percent.as_deref(), Some("80"));
        assert_eq!(quotas[1].remaining.as_deref(), Some("7994"));
    }

    #[test]
    fn zai_api_bases_are_normalized_to_the_monitor_origin() {
        assert_eq!(
            zai_origin("https://api.z.ai/api/coding/paas/v4/"),
            "https://api.z.ai"
        );
        assert_eq!(
            zai_origin("https://api.z.ai/api/paas/v4"),
            "https://api.z.ai"
        );
        assert_eq!(zai_origin("https://api.z.ai"), "https://api.z.ai");
    }

    #[test]
    fn openai_costs_sum_without_floating_point() {
        let spent = parse_openai_costs(&json!({
            "data": [
                {"results": [{"amount": {"value": 0.06, "currency": "usd"}}]},
                {"results": [{"amount": {"value": "1.234567", "currency": "usd"}}]}
            ]
        }))
        .unwrap();
        assert_eq!(spent.render(), "1.294567");
    }

    #[test]
    fn utc_month_start_handles_leap_year() {
        // 2024-02-29T12:00:00Z -> 2024-02-01T00:00:00Z.
        assert_eq!(start_of_month_utc_at(1_709_208_000), 1_706_745_600);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }
}
