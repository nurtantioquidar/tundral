import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { ConsoleShell } from "@/components/ConsoleShell";
import { ProviderTable, type ProviderSummary } from "@/features/upstreams/UpstreamForms";
import { ADMIN_LINKS, AdminSignOut, UnavailableNotice } from "@/features/upstreams/shared";
import { GatewayError, gatewayAdmin } from "@/server/gateway-client";
import { readAllCapabilities } from "@/server/upstream-reads";
import { hasAdminSession } from "@/server/admin-session";

export const metadata: Metadata = { title: "Providers — Inver operator console" };

export default async function ProvidersPage() {
  if (!(await hasAdminSession())) redirect("/admin/signin");

  let listing;
  try {
    listing = await gatewayAdmin.listUpstreams();
  } catch (cause) {
    if (cause instanceof GatewayError && (cause.status === 409 || cause.status === 503)) {
      return (
        <ConsoleShell area="Operator" links={ADMIN_LINKS.providers} actions={<AdminSignOut />}>
          <div className="console-head">
            <div>
              <h1 className="console-head__title">Providers</h1>
              <p className="console-head__sub">The provider pool is unavailable right now.</p>
            </div>
          </div>
          <UnavailableNotice message={cause.message} />
        </ConsoleShell>
      );
    }
    throw cause;
  }

  /* Two reads total. Model counts come from the single bulk capability read rather than one request
   * per provider. Key counts are deliberately not shown here — credentials are per-provider only, and
   * fetching them for a list would restore the fan-out this page exists to remove. */
  const capabilityRead = await readAllCapabilities(listing.data.map((provider) => provider.id));
  const modelCounts = capabilityRead.capabilities.reduce<Record<string, number>>((counts, capability) => {
    counts[capability.provider_id] = (counts[capability.provider_id] ?? 0) + 1;
    return counts;
  }, {});

  const summaries: ProviderSummary[] = listing.data.map((provider) => ({
    id: provider.id,
    name: provider.name,
    transport: provider.transport,
    enabled: provider.enabled,
    baseUrl: provider.base_url,
    maxConcurrency: provider.max_concurrency,
    modelCount: modelCounts[provider.id] ?? 0,
  }));

  return (
    <ConsoleShell area="Operator" links={ADMIN_LINKS.providers} actions={<AdminSignOut />}>
      <div className="console-head">
        <div>
          <h1 className="console-head__title">Providers</h1>
          <p className="console-head__sub">
            Each provider declares which models it can serve. Open one to manage its keys, models, and
            concurrency.
          </p>
        </div>
        <Link className="btn btn--primary" href="/admin/upstreams/providers/new">Add provider</Link>
      </div>

      {listing.runtime.applied_from_database ? null : (
        <div className="notice notice--warn">
          The gateway is routing from environment configuration. Providers here are saved, but do not
          affect live traffic until the deployment sets <code>UPSTREAM_CONFIG_SOURCE=database</code>{" "}
          and restarts.
        </div>
      )}

      {capabilityRead.degraded ? (
        <div className="notice notice--warn">
          This gateway does not serve <code>GET /internal/upstream-models</code> yet, so model counts
          were read one request per provider. Restart the gateway on a build that includes it.
        </div>
      ) : null}

      <ProviderTable providers={summaries} />
    </ConsoleShell>
  );
}
