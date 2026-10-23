import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { ConsoleShell } from "@/components/ConsoleShell";
import {
  AddCapabilityForm,
  AddKeyForm,
  CapabilityRow,
  KeyRow,
  ModelPicker,
  ProviderControls,
} from "@/features/upstreams/UpstreamForms";
import {
  ADMIN_LINKS,
  AdminSignOut,
  SetupChecklist,
  UnavailableNotice,
  canonicalModelId,
  type SetupStep,
} from "@/features/upstreams/shared";
import { GatewayError, gatewayAdmin } from "@/server/gateway-client";
import { readRateCard } from "@/server/upstream-reads";
import { hasAdminSession } from "@/server/admin-session";
import { day } from "@/lib/format";

export const metadata: Metadata = { title: "Provider — Inver operator console" };

const routeHref = (publicModelId: string) =>
  `/admin/upstreams?model=${encodeURIComponent(publicModelId)}#model-${encodeURIComponent(publicModelId)}`;

export default async function ProviderDetailPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ created?: string }>;
}) {
  if (!(await hasAdminSession())) redirect("/admin/signin");
  const [{ id }, { created }] = await Promise.all([params, searchParams]);

  let listing;
  try {
    listing = await gatewayAdmin.listUpstreams();
  } catch (cause) {
    if (cause instanceof GatewayError && (cause.status === 409 || cause.status === 503)) {
      return (
        <ConsoleShell area="Operator" links={ADMIN_LINKS.providers} actions={<AdminSignOut />}>
          <div className="console-head">
            <div><h1 className="console-head__title">Provider</h1></div>
          </div>
          <UnavailableNotice message={cause.message} />
        </ConsoleShell>
      );
    }
    throw cause;
  }

  const provider = listing.data.find((candidate) => candidate.id === id);
  if (!provider) notFound();
  const isCodex = provider.transport === "codex_cli";

  /* Only this provider's collections, plus the route list (bounded by the rate card) so the checklist
   * can say whether any published route actually sends traffic here. Codex providers cannot hold
   * credentials at all — the gateway rejects them — so that read is skipped rather than returning an
   * empty list. */
  const [credentials, capabilities, rateCardRead, routes] = await Promise.all([
    isCodex ? Promise.resolve([]) : gatewayAdmin.listUpstreamCredentials(provider.id),
    gatewayAdmin.listUpstreamModels(provider.id),
    readRateCard(),
    gatewayAdmin.listModelRoutes(),
  ]);
  /* Group customer-facing models by the canonical model they resolve to, so each capability is only
   * offered the public models the gateway will actually accept for a test. */
  const publicByCanonical = new Map<string, string[]>();
  for (const model of rateCardRead.card.data) {
    const canonical = canonicalModelId(model);
    publicByCanonical.set(canonical, [...(publicByCanonical.get(canonical) ?? []), model.id]);
  }

  /* Registration progress, derived entirely from authoritative gateway reads. Stored routes for
   * models the rate card no longer sells (retired aliases) are dormant in the gateway and must not
   * count as serving traffic here. */
  const sellablePublicIds = new Set(rateCardRead.card.data.map((model) => model.id));
  const capabilityIds = new Set(capabilities.map((capability) => capability.id));
  const routedModels = routes
    .filter((route) =>
      sellablePublicIds.has(route.public_model_id) &&
      route.targets.some((target) => capabilityIds.has(target.capability_id)))
    .map((route) => route.public_model_id);
  const compatibleModels = capabilities.flatMap(
    (capability) => publicByCanonical.get(capability.canonical_model_id) ?? [],
  );
  const unroutedModels = [...new Set(compatibleModels)].filter((model) => !routedModels.includes(model));
  const tested = capabilities.some((capability) => capability.verified_at !== null);

  const steps: SetupStep[] = [
    {
      key: "provider",
      label: "Register the provider",
      state: "done",
      hint: `Added ${day(provider.created_at)}. Connection and concurrency live in the panel below.`,
    },
    isCodex
      ? {
          key: "key",
          label: "Sign in on the gateway host",
          state: "manual",
          hint: (
            <>Run <code>codex login</code> once on the machine running the gateway — there is no key to store here.</>
          ),
        }
      : {
          key: "key",
          label: "Store an API key",
          state: credentials.length > 0 ? "done" : "todo",
          hint:
            credentials.length > 0
              ? `${credentials.length} key${credentials.length === 1 ? "" : "s"} stored, write-only and encrypted.`
              : "The provider cannot authenticate upstream until at least one key is stored.",
          ...(credentials.length === 0 ? { action: { href: "#keys", label: "Add a key" } } : {}),
        },
    {
      key: "models",
      label: "Declare the models it can serve",
      state: capabilities.length > 0 ? "done" : "todo",
      hint:
        capabilities.length > 0
          ? `${capabilities.length} model${capabilities.length === 1 ? "" : "s"} declared.`
          : isCodex || credentials.length === 0
            ? "Declare which of the provider’s models Inver should serve."
            : "Load the provider’s own model list below and tick the ones Inver should serve.",
      ...(capabilities.length === 0 ? { action: { href: "#models", label: "Add a model" } } : {}),
    },
    {
      key: "test",
      label: "Test a declared model",
      state: tested ? "done" : "recommended",
      hint: tested
        ? "At least one model returned a real generation."
        : "Runs one real generation against the provider and can spend its credit.",
      ...(!tested && capabilities.length > 0 ? { action: { href: "#models", label: "Run a test" } } : {}),
    },
    {
      key: "route",
      label: "Publish a route",
      state: routedModels.length > 0 ? "done" : "todo",
      hint:
        routedModels.length > 0
          ? `Serving ${routedModels.join(", ")}${unroutedModels.length > 0 ? ` — not yet routed for ${unroutedModels.join(", ")}` : ""}.`
          : capabilities.length === 0
            ? "Available once a model is declared."
            : unroutedModels.length === 0
              ? "No customer-facing model resolves to this provider’s canonical model IDs — check their spelling above."
              : `Customer requests reach this provider only through a published route. Compatible: ${unroutedModels.join(", ")}.`,
      ...(unroutedModels.length > 0
        ? { action: { href: routeHref(unroutedModels[0]), label: "Open Routing" } }
        : {}),
    },
  ];

  return (
    <ConsoleShell area="Operator" links={ADMIN_LINKS.providers} actions={<AdminSignOut />}>
      <nav className="crumbs" aria-label="Breadcrumb">
        <Link href="/admin/upstreams/providers">Providers</Link>
        <span aria-hidden="true">/</span>
        <span aria-current="page">{provider.name}</span>
      </nav>

      <div className="console-head">
        <div>
          <h1 className="console-head__title">{provider.name}</h1>
          <p className="console-head__sub">
            {isCodex ? "Codex subscription" : "OpenAI-compatible HTTP"} · revision {provider.revision} ·
            added {day(provider.created_at)}
          </p>
        </div>
        {provider.enabled ? <span className="chip chip--ok">enabled</span> : <span className="chip chip--error">disabled</span>}
      </div>

      {created === "live" ? (
        <div className="notice notice--ok">
          Provider created and applied to the running pool. Finish the checklist below before it can
          serve customer traffic.
        </div>
      ) : created === "staged" ? (
        <div className="notice notice--warn">
          Provider created and stored, but the gateway still routes from environment configuration.
          Finish the checklist below; it goes live once the deployment sets{" "}
          <code>UPSTREAM_CONFIG_SOURCE=database</code> and restarts.
        </div>
      ) : null}

      <SetupChecklist steps={steps} />

      <section className="panel">
        <div className="panel__head"><h2 className="panel__title">Connection</h2></div>
        <div className="panel__body stack">
          {isCodex ? (
            <div className="explain">
              <p className="explain__title">Codex signs in on the gateway host, not here.</p>
              <p className="explain__body">
                This transport runs the local <code>codex</code> binary, which carries its own
                subscription login. There is no URL and no API key to store — and nothing this console
                can click to authenticate. Sign in once on the machine running the gateway:
              </p>
              <pre className="explain__cmd"><code>codex login</code></pre>
              <p className="explain__body">
                The gateway reports whether it can find the <code>codex</code> binary in the routing
                status. It deliberately never reads the stored credential.
              </p>
            </div>
          ) : (
            <div className="kv">
              <span className="meta">Base URL</span>
              <span className="kv__value mono">{provider.base_url}</span>
            </div>
          )}
          <ProviderControls provider={provider} />
        </div>
      </section>

      {isCodex ? null : (
        <section className="panel" id="keys">
          <div className="panel__head">
            <h2 className="panel__title">API keys</h2>
            <span className="meta">{credentials.length} stored</span>
          </div>
          <div className="panel__body stack">
            {credentials.length === 0 ? (
              <p className="field__help">
                No keys yet — this provider cannot serve traffic until one is added.
              </p>
            ) : (
              <ul className="rowlist">
                {credentials.map((credential) => (
                  <KeyRow key={credential.id} credential={credential} lastKey={credentials.length === 1} />
                ))}
              </ul>
            )}
            {/* The next undone step arrives with its form already open. */}
            <details className="sub" open={credentials.length === 0}>
              <summary className="sub__summary">Add a key</summary>
              <div className="sub__body"><AddKeyForm provider={provider} /></div>
            </details>
          </div>
        </section>
      )}

      <section className="panel" id="models">
        <div className="panel__head">
          <h2 className="panel__title">Models this provider can serve</h2>
          <span className="meta">{capabilities.length} declared</span>
        </div>
        <div className="panel__body stack">
          {capabilities.length === 0 ? (
            <p className="field__help">
              None yet. This provider only becomes selectable in a route once it declares a matching
              model.
            </p>
          ) : (
            <ul className="rowlist">
              {capabilities.map((capability) => (
                <CapabilityRow
                  key={capability.id}
                  capability={capability}
                  testableModelIds={publicByCanonical.get(capability.canonical_model_id) ?? []}
                />
              ))}
            </ul>
          )}
          <p className="field__help">
            Testing runs one real generation against the provider and can spend its credit.
            {rateCardRead.degraded
              ? " This gateway does not serve GET /internal/rate-card yet, so only models that are already routable can be tested."
              : ""}
          </p>
          {/* Picking beats typing: with a key stored, the provider itself says what it can serve.
            * Codex has no /v1/models to ask, and without a key there is nothing to authenticate
            * discovery with — those cases fall back to the manual form. */}
          {!isCodex && credentials.length > 0 ? (
            <ModelPicker
              provider={provider}
              sellableModels={[...publicByCanonical.keys()]}
              declaredUpstreamIds={capabilities.map((capability) => capability.upstream_model_id)}
            />
          ) : null}
          {!isCodex && credentials.length === 0 ? (
            <p className="field__help">
              Store an API key first to pick from the provider&rsquo;s own model list.
            </p>
          ) : null}
          <details className="sub" open={isCodex && capabilities.length === 0}>
            <summary className="sub__summary">Add a model manually</summary>
            <div className="sub__body"><AddCapabilityForm provider={provider} /></div>
          </details>
        </div>
      </section>

      <p className="field__help">
        Ordering providers for each customer-facing model happens in{" "}
        <Link className="link-cta" href="/admin/upstreams">Routing</Link>.
      </p>
    </ConsoleShell>
  );
}
