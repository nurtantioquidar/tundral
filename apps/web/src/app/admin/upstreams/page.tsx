import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { ConsoleShell } from "@/components/ConsoleShell";
import { RouteEditor, type RouteCandidate } from "@/features/upstreams/UpstreamForms";
import { ADMIN_LINKS, AdminSignOut, RuntimeStrip, UnavailableNotice, canonicalModelId } from "@/features/upstreams/shared";
import type { ModelRoute } from "@monet/contracts/upstreams";
import { GatewayError, gatewayAdmin } from "@/server/gateway-client";
import { readAllCapabilities, readRateCard } from "@/server/upstream-reads";
import { hasAdminSession } from "@/server/admin-session";

export const metadata: Metadata = { title: "Model routing — Monet operator console" };

export default async function RoutingPage({
  searchParams,
}: {
  searchParams: Promise<{ model?: string }>;
}) {
  if (!(await hasAdminSession())) redirect("/admin/signin");
  /* A provider page links here with ?model= so the relevant editor arrives already open. */
  const { model: focusModelId } = await searchParams;

  let listing;
  try {
    listing = await gatewayAdmin.listUpstreams();
  } catch (cause) {
    if (cause instanceof GatewayError && (cause.status === 409 || cause.status === 503)) {
      return (
        <ConsoleShell area="Operator" links={ADMIN_LINKS.routing} actions={<AdminSignOut />}>
          <div className="console-head">
            <div>
              <h1 className="console-head__title">Model routing</h1>
              <p className="console-head__sub">The routing editor is unavailable right now.</p>
            </div>
          </div>
          <UnavailableNotice message={cause.message} />
        </ConsoleShell>
      );
    }
    throw cause;
  }

  /* Four reads, whatever the provider count. Credentials are deliberately absent — they only matter
   * on a provider's own page, and fetching them here would reintroduce a per-provider fan-out. */
  const providers = listing.data;
  const [capabilityRead, routes, rateCardRead] = await Promise.all([
    readAllCapabilities(providers.map((provider) => provider.id)),
    gatewayAdmin.listModelRoutes(),
    readRateCard(),
  ]);
  const capabilities = capabilityRead.capabilities;
  const providersById = new Map(providers.map((provider) => [provider.id, provider]));
  const routesByModel = new Map<string, ModelRoute>(routes.map((route) => [route.public_model_id, route]));

  const candidates: RouteCandidate[] = capabilities.map((capability) => {
    const provider = providersById.get(capability.provider_id);
    return {
      capabilityId: capability.id,
      providerName: provider?.name ?? "Unknown provider",
      providerEnabled: provider?.enabled ?? false,
      transport: provider?.transport ?? "openai_http",
      upstreamModelId: capability.upstream_model_id,
      apiPaths: capability.api_paths,
      capabilityEnabled: capability.enabled,
    };
  });
  const canonicalByCapability = new Map(capabilities.map((c) => [c.id, c.canonical_model_id]));
  const routedCount = rateCardRead.card.data.filter((model) => (routesByModel.get(model.id)?.targets.length ?? 0) > 0).length;

  return (
    <ConsoleShell area="Operator" links={ADMIN_LINKS.routing} actions={<AdminSignOut />}>
      <div className="console-head">
        <div>
          <h1 className="console-head__title">Model routing</h1>
          <p className="console-head__sub">
            Each customer-facing model gets an ordered list of providers to try. A model with no route
            is rejected rather than sent somewhere else.
          </p>
        </div>
      </div>

      <RuntimeStrip runtime={listing.runtime} />

      {listing.runtime.applied_from_database ? null : (
        <div className="notice notice--warn">
          The gateway is routing from environment configuration. Changes here are saved, but will not
          affect live traffic until the deployment sets <code>UPSTREAM_CONFIG_SOURCE=database</code>{" "}
          and restarts.
        </div>
      )}

      {rateCardRead.degraded ? (
        <div className="notice notice--warn">
          This gateway does not serve <code>GET /internal/rate-card</code> yet, so only models that are
          already routable are listed. A model awaiting its first route is therefore missing from this
          screen — restart the gateway on a build that includes the endpoint to configure it.
        </div>
      ) : null}

      {capabilityRead.degraded ? (
        <div className="notice notice--warn">
          This gateway does not serve <code>GET /internal/upstream-models</code> yet, so capabilities
          were read one request per provider. Restart the gateway on a build that includes it to make
          this page a fixed number of reads regardless of pool size.
        </div>
      ) : null}

      <div className="crossref">
        <span>
          <strong>{providers.length}</strong> provider{providers.length === 1 ? "" : "s"} supplying{" "}
          <strong>{capabilities.length}</strong> model capabilit{capabilities.length === 1 ? "y" : "ies"}.
        </span>
        <Link className="link-cta" href="/admin/upstreams/providers">
          Manage providers
        </Link>
      </div>

      <section className="block">
        <div className="block__head">
          <h2 className="block__title">Models</h2>
          <span className="meta">
            {routedCount} of {rateCardRead.card.data.length} routed · rate card {rateCardRead.card.rate_card_version}
          </span>
        </div>
        <ul className="disclist">
          {rateCardRead.card.data.map((model) => {
            const canonical = canonicalModelId(model);
            const route = routesByModel.get(model.id);
            const compatible = candidates.filter(
              (candidate) => canonicalByCapability.get(candidate.capabilityId) === canonical,
            );
            const targets = (route?.targets ?? []).filter((target) =>
              compatible.some((candidate) => candidate.capabilityId === target.capability_id));
            const hasTie = targets.some((target, index, all) =>
              all.findIndex((other) => other.priority === target.priority) !== index);
            return (
              <li key={model.id} id={`model-${model.id}`}>
                <details className="disc" open={model.id === focusModelId || undefined}>
                  <summary className="disc__summary">
                    <span className="disc__title mono">{model.id}</span>
                    {targets.length > 0 ? (
                      <span className="chip chip--ok">{targets.length} provider{targets.length === 1 ? "" : "s"}</span>
                    ) : (
                      <span className="chip chip--error">no route</span>
                    )}
                    <span className="disc__meta meta">
                      {model.reasoning_effort ? `reasoning ${model.reasoning_effort} · ` : ""}
                      canonical {canonical}
                      {route ? ` · rev ${route.revision}` : ""}
                    </span>
                  </summary>
                  <div className="disc__body">
                    {hasTie ? (
                      <div className="notice notice--warn">
                        This route splits traffic inside a priority tier. The tie is preserved below as
                        &ldquo;split traffic with the row above&rdquo; — leave it checked to keep the split.
                      </div>
                    ) : null}
                    <RouteEditor
                      publicModelId={model.id}
                      revision={route?.revision ?? 0}
                      initial={targets.map((target) => ({
                        capabilityId: target.capability_id,
                        priority: target.priority,
                        weight: target.weight,
                      }))}
                      candidates={compatible}
                    />
                  </div>
                </details>
              </li>
            );
          })}
        </ul>
      </section>
    </ConsoleShell>
  );
}
