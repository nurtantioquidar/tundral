import type { ReactNode } from "react";
import { adminSignOut } from "@/app/admin/actions";
import type { PricedModel } from "@tundral/contracts";
import type { UpstreamRuntime } from "@tundral/contracts/upstreams";

/* Console navigation for the operator surfaces. Routing and providers are separate destinations
 * because they scale differently: the model list is bounded by the rate card, the provider list is
 * not. */
export const ADMIN_LINKS = {
  routing: [
    { href: "/admin", label: "Overview" },
    { href: "/admin/upstreams", label: "Routing", current: true },
    { href: "/admin/upstreams/providers", label: "Providers" },
  ],
  providers: [
    { href: "/admin", label: "Overview" },
    { href: "/admin/upstreams", label: "Routing" },
    { href: "/admin/upstreams/providers", label: "Providers", current: true },
  ],
};

export function AdminSignOut() {
  return (
    <form action={adminSignOut}>
      <button type="submit" className="btn btn--ghost">Sign out</button>
    </form>
  );
}

/* Current gateways sell models under their canonical ID, so this is the identity function. The
 * suffix-stripping branch remains for gateways predating rate card 2026-08-02.1, whose retired
 * `gpt-5.6-luna-low`/`-medium` aliases carried a `reasoning_effort` and resolved to canonical
 * `gpt-5.6-luna`. A route may only target capabilities whose canonical model matches. */
export function canonicalModelId(model: PricedModel): string {
  const suffix = model.reasoning_effort ? `-${model.reasoning_effort}` : "";
  return suffix && model.id.endsWith(suffix) ? model.id.slice(0, -suffix.length) : model.id;
}

/* One registration step on the provider go-live checklist. `manual` marks work that happens on the
 * gateway host (Codex sign-in) which this console can neither perform nor verify; `recommended`
 * marks steps that are skippable but wise (a real provider test). */
export interface SetupStep {
  key: string;
  label: string;
  state: "done" | "todo" | "manual" | "recommended";
  hint: ReactNode;
  action?: { href: string; label: string };
}

const STEP_CHIP: Record<SetupStep["state"], { className: string; label: string }> = {
  done: { className: "chip chip--ok", label: "done" },
  todo: { className: "chip chip--muted", label: "to do" },
  manual: { className: "chip chip--muted", label: "on the host" },
  recommended: { className: "chip chip--muted", label: "recommended" },
};

export function SetupChecklist({ steps }: { steps: SetupStep[] }) {
  const done = steps.filter((step) => step.state === "done").length;
  return (
    <section className="setup" aria-labelledby="setup-title">
      <div className="setup__head">
        <h2 id="setup-title" className="setup__title">Go-live checklist</h2>
        <span className="meta tnum">{done} of {steps.length} done</span>
      </div>
      <ol className="setup__list">
        {steps.map((step, index) => {
          const chip = STEP_CHIP[step.state];
          return (
            <li key={step.key} className={`setup__step${step.state === "done" ? " setup__step--done" : ""}`}>
              <span className="setup__mark tnum" aria-hidden="true">{step.state === "done" ? "✓" : index + 1}</span>
              <span className="setup__body">
                <span className="setup__name">
                  {step.label} <span className={chip.className}>{chip.label}</span>
                </span>
                <span className="setup__hint meta">{step.hint}</span>
              </span>
              {step.action ? (
                <a className="link-cta setup__action" href={step.action.href}>{step.action.label}</a>
              ) : null}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export function UnavailableNotice({ message }: { message: string }) {
  /* Two distinct operational states share this notice: the gateway process being down entirely,
   * and a reachable gateway running without database-backed routing. Each gets its own next step. */
  const unreachable = message.includes("could not be reached");
  return (
    <div className="notice notice--error">
      {message}.{" "}
      {unreachable ? (
        <>Start the gateway (or fix <code>GATEWAY_URL</code>), then reload this page.</>
      ) : (
        <>
          Start the gateway with <code>UPSTREAM_CONFIG_SOURCE=database</code> and a stable{" "}
          <code>UPSTREAM_CREDENTIAL_ENCRYPTION_KEY</code>, then reload this page.
        </>
      )}
    </div>
  );
}

export function RuntimeStrip({ runtime }: { runtime: UpstreamRuntime }) {
  const databaseRouting = runtime.routes !== undefined;
  return (
    <dl className="statstrip">
      <div className="statstrip__cell">
        <dt className="meta">Config source</dt>
        <dd>
          <span className={runtime.applied_from_database ? "chip chip--ok" : "chip chip--muted"}>
            {runtime.configuration_source}
          </span>
        </dd>
      </div>
      <div className="statstrip__cell">
        <dt className="meta">Routing</dt>
        <dd>
          <span className={runtime.configured ? "chip chip--ok" : "chip chip--error"}>
            {runtime.configured ? "ready" : "not routable"}
          </span>
        </dd>
      </div>
      <div className="statstrip__cell">
        <dt className="meta">{databaseRouting ? "Routes / targets" : "Keys / providers"}</dt>
        <dd className="statstrip__value tnum">
          {databaseRouting
            ? `${runtime.routes ?? 0} / ${runtime.targets ?? 0}`
            : `${runtime.available ?? 0} / ${runtime.providers ?? 0}`}
        </dd>
      </div>
      <div className="statstrip__cell">
        <dt className="meta">{databaseRouting ? "Available now" : "Capacity"}</dt>
        <dd className="statstrip__value tnum">
          {databaseRouting ? (runtime.availableTargets ?? 0) : (runtime.capacity ?? 0)}
        </dd>
      </div>
      <div className="statstrip__cell">
        <dt className="meta">In flight</dt>
        <dd className="statstrip__value tnum">{runtime.inflight}</dd>
      </div>
    </dl>
  );
}
