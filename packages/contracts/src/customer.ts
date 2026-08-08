export interface CustomerRow {
  id: string;
  name: string;
  email: string | null;
  active: number;
  daily_spend_limit_micros: number | null;
  per_request_limit_micros: number | null;
  created_at: number;
  updated_at: number;
  posted_balance_micros: number;
  active_key_count: number;
}

export interface CustomerAccount {
  customerId: string;
  name: string;
  email: string;
}

export interface RegisterCustomerAccountRequest {
  name: string;
  email: string;
  password: string;
}

export interface AuthenticateCustomerAccountRequest {
  email: string;
  password: string;
}

export interface UpdateCustomerRequest {
  active?: boolean;
  daily_spend_limit_usd?: string | null;
  per_request_limit_usd?: string | null;
}

export interface CreateApiKeyRequest {
  name: string;
  spend_limit_usd?: string | null;
}

export interface UpdateApiKeyRequest {
  name?: string;
  spend_limit_usd?: string | null;
}

export interface CreditCustomerRequest {
  amount_usd: string;
  kind: string;
  reference: string;
  note?: string;
}

export interface RegisteredCustomerAccount extends CustomerAccount {
  apiKeyId: string;
  apiKey: string;
}

export interface CreatedCustomer {
  customerId: string;
  apiKeyId: string;
  apiKey: string;
}

export interface CreatedApiKey {
  apiKeyId: string;
  apiKey: string;
  keyPrefix: string;
}

export interface ApiKeyRow {
  id: string;
  key_prefix: string;
  name: string;
  active: number;
  max_concurrency: number;
  rate_limit_per_minute: number;
  spend_limit_micros: number | null;
  spent_micros: number;
  active_holds_micros: number;
  expires_at: number | null;
  created_at: number;
  revoked_at: number | null;
}

export interface UsageEvent {
  request_id: string;
  api_key_id: string;
  api_key_name: string;
  model: string;
  rate_card_version: string;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_tokens: number;
  output_tokens: number;
  cost_micros: number;
  balance_before_micros: number | null;
  balance_after_micros: number | null;
  upstream_status: number;
  created_at: number;
}

export interface UsageTotals {
  requests: number;
  input_tokens: number;
  cached_input_tokens: number;
  output_tokens: number;
  cost_micros: number;
}

export interface UsageSummary {
  days: number;
  since: number;
  daily: Array<UsageTotals & { day: string }>;
  models: Array<UsageTotals & { model: string }>;
  totals: UsageTotals;
}

export interface Reconciliation {
  customerId: string;
  postedBalanceMicros: number;
  activeHoldsMicros: number;
  availableBalanceMicros: number;
  totalCreditsMicros: number;
  totalDebitsMicros: number;
  usageLedgerDebitsMicros: number;
  usageChargesMicros: number;
  usageCount: number;
  missingUsageLedgerEntries: number;
  reconciled: boolean;
}

export interface CreditResult {
  replayed: boolean;
  creditedMicros: string;
  balance_usd: string;
}
