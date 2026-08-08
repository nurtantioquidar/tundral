export interface PricedModel {
  id: string;
  reasoning_effort?: "low" | "medium";
  pricing: {
    currency: string;
    unit: string;
    input: string;
    cache_write: string;
    cache_read: string;
    output: string;
  };
}

export interface PricingResponse {
  rate_card_version: string;
  currency: "USD";
  unit: "million_tokens";
  data: PricedModel[];
}
