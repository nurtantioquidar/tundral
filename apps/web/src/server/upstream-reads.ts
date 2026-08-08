import "server-only";
import type { PricingResponse } from "@monet/contracts";
import type { UpstreamModelCapability } from "@monet/contracts/upstreams";
import { GatewayError, gatewayAdmin } from "./gateway-client";

export interface RateCardRead {
  card: PricingResponse;
  /** True when only the routable subset was available, so unrouted models are missing. */
  degraded: boolean;
}

/* The full rate card, including models that have no route yet.
 *
 * On a gateway that predates `GET /internal/rate-card`, fall back to `/internal/pricing`. That view is
 * narrowed to routable models, so a model awaiting its first route stays invisible — the caller must
 * surface `degraded` rather than let the operator conclude the model does not exist. */
export async function readRateCard(): Promise<RateCardRead> {
  try {
    return { card: await gatewayAdmin.rateCard(), degraded: false };
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 404) {
      return { card: await gatewayAdmin.pricing(), degraded: true };
    }
    throw cause;
  }
}

export interface CapabilityRead {
  capabilities: UpstreamModelCapability[];
  /** True when the bulk endpoint was missing and the slow per-provider path was used instead. */
  degraded: boolean;
}

/* Reads every model capability in one request.
 *
 * The apps deploy independently, so the web app can run against a gateway that predates
 * `GET /internal/upstream-models`. Rather than breaking the operator console during that window, fall
 * back to the original one-request-per-provider fan-out and report it, so the page still works and the
 * operator learns why it is slow. */
export async function readAllCapabilities(providerIds: string[]): Promise<CapabilityRead> {
  try {
    return { capabilities: await gatewayAdmin.listAllUpstreamModels(), degraded: false };
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 404) {
      const groups = await Promise.all(providerIds.map((id) => gatewayAdmin.listUpstreamModels(id)));
      return { capabilities: groups.flat(), degraded: true };
    }
    throw cause;
  }
}
