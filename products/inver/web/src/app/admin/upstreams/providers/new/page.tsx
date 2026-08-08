import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { ConsoleShell } from "@/components/ConsoleShell";
import { AddProviderForm } from "@/features/upstreams/UpstreamForms";
import { ADMIN_LINKS, AdminSignOut, UnavailableNotice } from "@/features/upstreams/shared";
import { GatewayError, gatewayAdmin } from "@/server/gateway-client";
import { hasAdminSession } from "@/server/admin-session";

export const metadata: Metadata = { title: "Add provider — Monet operator console" };

/* Registration is a journey, not a single form: the provider created here cannot serve customer
 * traffic until it also has a credential, declared models, and a published route. This page states
 * that up front, and the create action continues onto the provider's own page where those steps
 * live. */
const NEXT_STEPS = [
  { name: "Store an API key", detail: "write-only, encrypted at rest — Codex providers sign in on the gateway host instead" },
  { name: "Declare its models", detail: "map Monet’s canonical model IDs to the provider’s own model IDs" },
  { name: "Publish a route", detail: "order providers per customer-facing model in Routing" },
];

export default async function NewProviderPage() {
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
              <h1 className="console-head__title">Add provider</h1>
              <p className="console-head__sub">Providers cannot be added right now.</p>
            </div>
          </div>
          <UnavailableNotice message={cause.message} />
        </ConsoleShell>
      );
    }
    throw cause;
  }

  return (
    <ConsoleShell area="Operator" links={ADMIN_LINKS.providers} actions={<AdminSignOut />}>
      <nav className="crumbs" aria-label="Breadcrumb">
        <Link href="/admin/upstreams/providers">Providers</Link>
        <span aria-hidden="true">/</span>
        <span aria-current="page">Add provider</span>
      </nav>

      <div className="console-head">
        <div>
          <h1 className="console-head__title">Add provider</h1>
          <p className="console-head__sub">
            Step 1 of 4 — a new provider carries no traffic until it has a key, declared models, and a
            published route. Creating it takes you straight to those steps.
          </p>
        </div>
      </div>

      {listing.runtime.applied_from_database ? null : (
        <div className="notice notice--warn">
          The gateway is routing from environment configuration. This provider will be saved, but will
          not affect live traffic until the deployment sets <code>UPSTREAM_CONFIG_SOURCE=database</code>{" "}
          and restarts.
        </div>
      )}

      <section className="panel">
        <div className="panel__head"><h2 className="panel__title">Provider</h2></div>
        <div className="panel__body">
          <AddProviderForm />
        </div>
      </section>

      <section className="setup" aria-labelledby="next-steps-title">
        <div className="setup__head">
          <h2 id="next-steps-title" className="setup__title">After this</h2>
        </div>
        <ol className="setup__list">
          {NEXT_STEPS.map((step, index) => (
            <li key={step.name} className="setup__step">
              <span className="setup__mark tnum" aria-hidden="true">{index + 2}</span>
              <span className="setup__body">
                <span className="setup__name">{step.name}</span>
                <span className="setup__hint meta">{step.detail}</span>
              </span>
            </li>
          ))}
        </ol>
      </section>
    </ConsoleShell>
  );
}
