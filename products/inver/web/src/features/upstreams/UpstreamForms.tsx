"use client";

import Link from "next/link";
import { useActionState, useEffect, useMemo, useRef, useState } from "react";
import {
  addDiscoveredModels,
  createUpstream,
  createUpstreamCredential,
  createUpstreamModel,
  deleteUpstream,
  deleteUpstreamKey,
  deleteUpstreamModel,
  discoverProviderModels,
  publishModelRoute,
  rotateUpstreamKey,
  setUpstreamEnabled,
  testUpstream,
  updateUpstreamSettings,
  type ActionState,
} from "@/app/admin/actions";
import type {
  UpstreamApiPath,
  UpstreamCredential,
  UpstreamModelCapability,
  UpstreamProvider,
  UpstreamTransport,
} from "@monet/contracts/upstreams";

function StatusLine({ state }: { state: ActionState }) {
  return (
    <p className={`field__help${state.error ? " field__help--error" : state.ok ? " field__help--ok" : ""}`} aria-live="polite">
      {state.error ?? state.ok ?? ""}
    </p>
  );
}

/* ---------- Provider list ---------- */

export interface ProviderSummary {
  id: string;
  name: string;
  transport: UpstreamTransport;
  enabled: boolean;
  baseUrl: string | null;
  maxConcurrency: number;
  modelCount: number;
}

/* Filtering stays client-side over the single provider read. That holds comfortably into the
 * hundreds; past that the query would move to the gateway's provider list, and this component is the
 * only place that would have to change. */
export function ProviderTable({ providers }: { providers: ProviderSummary[] }) {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const shown = needle
    ? providers.filter((provider) =>
        provider.name.toLowerCase().includes(needle) ||
        provider.transport.includes(needle) ||
        (provider.baseUrl ?? "").toLowerCase().includes(needle))
    : providers;

  if (providers.length === 0) {
    return (
      <div className="empty empty--inline">
        <p className="empty__title">No providers yet.</p>
        <p className="empty__body">
          <Link className="link-cta" href="/admin/upstreams/providers/new">Add the first provider</Link>, then
          declare which models it can serve.
        </p>
      </div>
    );
  }

  return (
    <div className="stack stack--sm">
      {/* A search field for six rows is clutter; it appears once scanning stops being trivial. */}
      {providers.length > 6 ? (
        <div className="field">
          <label className="field__label" htmlFor="provider-filter">Filter providers</label>
          <input
            id="provider-filter"
            className="input"
            type="search"
            value={query}
            placeholder="Name, transport, or URL"
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>
      ) : null}

      <div className="table-wrap">
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">Provider</th>
              <th scope="col">Transport</th>
              <th scope="col">Status</th>
              <th scope="col" className="num">Models</th>
              <th scope="col" className="num">Concurrency</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((provider) => (
              <tr key={provider.id}>
                <td>
                  <Link href={`/admin/upstreams/providers/${provider.id}`}>{provider.name}</Link>
                </td>
                <td>{provider.transport === "codex_cli" ? "Codex subscription" : "OpenAI HTTP"}</td>
                <td>
                  {provider.enabled
                    ? <span className="chip chip--ok">enabled</span>
                    : <span className="chip chip--muted">disabled</span>}
                </td>
                <td className="num">{provider.modelCount}</td>
                <td className="num">{provider.maxConcurrency}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <p className="field__help" aria-live="polite">
        {needle
          ? `${shown.length} of ${providers.length} provider${providers.length === 1 ? "" : "s"} match “${query.trim()}”.`
          : `${providers.length} provider${providers.length === 1 ? "" : "s"}.`}
      </p>
    </div>
  );
}

/* ---------- Providers ---------- */

export function AddProviderForm() {
  const [state, action, pending] = useActionState<ActionState, FormData>(createUpstream, {});
  const [transport, setTransport] = useState<UpstreamTransport>("openai_http");
  const formRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    if (state.ok) {
      formRef.current?.reset();
      setTransport("openai_http");
    }
  }, [state]);

  return (
    <form ref={formRef} action={action} className="stack">
      <div className="grid-2">
        <div className="field">
          <label className="field__label" htmlFor="np-name">Name</label>
          <input id="np-name" name="name" className="input" placeholder="Vendor A" maxLength={80} required disabled={pending} />
        </div>
        <div className="field">
          <label className="field__label" htmlFor="np-transport">Transport</label>
          <span className="select">
            <select
              id="np-transport"
              name="transport"
              className="input"
              value={transport}
              onChange={(event) => setTransport(event.target.value as UpstreamTransport)}
              disabled={pending}
            >
              <option value="openai_http">OpenAI-compatible HTTP</option>
              <option value="codex_cli">Codex subscription</option>
            </select>
          </span>
        </div>
      </div>

      {/* The gateway rejects base_url for codex_cli outright, so the field is not merely hidden — it
        * is replaced by the only thing that actually authorises that transport. */}
      {transport === "openai_http" ? (
        <div className="field">
          <label className="field__label" htmlFor="np-base-url">Base URL</label>
          <input
            id="np-base-url"
            name="base_url"
            className="input"
            type="url"
            placeholder="https://api.vendor.example/v1"
            required
            disabled={pending}
          />
          <p className="field__help">Must be HTTPS. Add the API key after the provider exists.</p>
        </div>
      ) : (
        <div className="explain">
          <p className="explain__title">Codex signs in on the gateway host, not here.</p>
          <p className="explain__body">
            This transport runs the local <code>codex</code> binary, which carries its own subscription
            login. There is no URL and no API key to store — and nothing this console can click to
            authenticate. Sign in once on the machine running the gateway:
          </p>
          <pre className="explain__cmd"><code>codex login</code></pre>
          <p className="explain__body">
            The gateway reports whether it can find the <code>codex</code> binary in the routing status
            on the Routing page. It deliberately never reads the stored credential.
          </p>
        </div>
      )}

      <div className="grid-2">
        <div className="field">
          <label className="field__label" htmlFor="np-conc">Max concurrency</label>
          <input id="np-conc" name="max_concurrency" className="input tnum" defaultValue="4" inputMode="numeric" disabled={pending} />
        </div>
        <label className="check">
          <input name="enabled" type="checkbox" defaultChecked disabled={pending} />
          <span>Enable straight away</span>
        </label>
      </div>

      <div className="form-actions">
        <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
          Add provider
        </button>
      </div>
      <StatusLine state={state} />
    </form>
  );
}

export function ProviderControls({ provider }: { provider: UpstreamProvider }) {
  const [toggleState, toggleAction, togglePending] = useActionState<ActionState, FormData>(setUpstreamEnabled, {});
  const [saveState, saveAction, savePending] = useActionState<ActionState, FormData>(updateUpstreamSettings, {});
  const [deleteState, deleteAction, deletePending] = useActionState<ActionState, FormData>(deleteUpstream, {});
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), 4000);
    return () => clearTimeout(timer);
  }, [armed]);

  return (
    <div className="stack stack--sm">
      <div className="form-actions">
        <form action={saveAction} className="inline-form">
          <input type="hidden" name="id" value={provider.id} />
          <input type="hidden" name="revision" value={provider.revision} />
          <label className="field__label" htmlFor={`conc-${provider.id}`}>Concurrency</label>
          <input
            id={`conc-${provider.id}`}
            name="max_concurrency"
            className="input input--narrow tnum"
            defaultValue={provider.max_concurrency}
            inputMode="numeric"
            disabled={savePending}
          />
          <button type="submit" className="btn btn--ghost" disabled={savePending}>Save</button>
        </form>

        <form action={toggleAction}>
          <input type="hidden" name="id" value={provider.id} />
          <input type="hidden" name="revision" value={provider.revision} />
          <input type="hidden" name="enabled" value={provider.enabled ? "false" : "true"} />
          <button type="submit" className="btn btn--ghost" disabled={togglePending}>
            {provider.enabled ? "Disable" : "Enable"}
          </button>
        </form>

        <form
          action={deleteAction}
          onSubmit={(event) => {
            if (!armed) {
              event.preventDefault();
              setArmed(true);
            }
          }}
        >
          <input type="hidden" name="id" value={provider.id} />
          <input type="hidden" name="revision" value={provider.revision} />
          <button type="submit" className="btn btn--danger" disabled={deletePending}>
            {armed ? "Confirm — delete provider" : "Delete"}
          </button>
        </form>
      </div>
      {armed ? (
        <p className="field__help field__help--error">
          This also deletes its API keys and models, and drops it from every published route.
        </p>
      ) : null}
      <StatusLine state={toggleState.error || toggleState.ok ? toggleState : saveState.error || saveState.ok ? saveState : deleteState} />
    </div>
  );
}

/* ---------- Keys ---------- */

export function AddKeyForm({ provider }: { provider: UpstreamProvider }) {
  const [state, action, pending] = useActionState<ActionState, FormData>(createUpstreamCredential, {});
  const formRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    if (state.ok) formRef.current?.reset();
  }, [state]);

  return (
    <form ref={formRef} action={action} className="stack stack--sm">
      <input type="hidden" name="provider_id" value={provider.id} />
      <div className="grid-2">
        <div className="field">
          <label className="field__label" htmlFor={`k-name-${provider.id}`}>Key name</label>
          <input id={`k-name-${provider.id}`} name="name" className="input" placeholder="primary" required disabled={pending} />
        </div>
        <div className="field">
          <label className="field__label" htmlFor={`k-secret-${provider.id}`}>API key</label>
          <input id={`k-secret-${provider.id}`} name="api_key" className="input" type="password" autoComplete="off" required disabled={pending} />
        </div>
      </div>
      <details className="sub">
        <summary className="sub__summary">Restrict models, weight, concurrency</summary>
        <div className="sub__body grid-3">
          <div className="field">
            <label className="field__label" htmlFor={`k-models-${provider.id}`}>Canonical models</label>
            <input id={`k-models-${provider.id}`} name="models" className="input" placeholder="all models" disabled={pending} />
          </div>
          <div className="field">
            <label className="field__label" htmlFor={`k-weight-${provider.id}`}>Weight</label>
            <input id={`k-weight-${provider.id}`} name="weight" className="input tnum" defaultValue="1" inputMode="numeric" disabled={pending} />
          </div>
          <div className="field">
            <label className="field__label" htmlFor={`k-conc-${provider.id}`}>Concurrency</label>
            <input id={`k-conc-${provider.id}`} name="max_concurrency" className="input tnum" defaultValue="4" inputMode="numeric" disabled={pending} />
          </div>
        </div>
      </details>
      <div className="form-actions">
        <button type="submit" className="btn btn--primary btn--sm" data-state={pending ? "loading" : undefined} disabled={pending}>
          Add key
        </button>
        <p className="field__help">Stored encrypted and never shown again.</p>
      </div>
      <StatusLine state={state} />
    </form>
  );
}

export function KeyRow({ credential, lastKey }: { credential: UpstreamCredential; lastKey: boolean }) {
  const [state, action, pending] = useActionState<ActionState, FormData>(rotateUpstreamKey, {});
  const [deleteState, deleteAction, deletePending] = useActionState<ActionState, FormData>(deleteUpstreamKey, {});
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), 4000);
    return () => clearTimeout(timer);
  }, [armed]);

  return (
    <li className="row">
      <div className="row__main">
        <span className="row__name">{credential.name}</span>
        <span className="row__meta meta">
          v{credential.credential_version} · weight {credential.weight} · conc {credential.max_concurrency}
          {credential.models ? ` · ${credential.models.join(", ")}` : " · all models"}
        </span>
      </div>
      <div className="row__actions">
        <details className="sub sub--inline">
          <summary className="sub__summary">Rotate</summary>
          <form action={action} className="sub__body inline-form">
            <input type="hidden" name="id" value={credential.id} />
            <input type="hidden" name="revision" value={credential.revision} />
            <label className="field__label" htmlFor={`rot-${credential.id}`}>New key</label>
            <input id={`rot-${credential.id}`} name="api_key" className="input" type="password" autoComplete="off" required disabled={pending} />
            <button type="submit" className="btn btn--ghost btn--sm" disabled={pending}>Replace</button>
          </form>
        </details>
        <form
          action={deleteAction}
          onSubmit={(event) => {
            if (!armed) {
              event.preventDefault();
              setArmed(true);
            }
          }}
        >
          <input type="hidden" name="id" value={credential.id} />
          <input type="hidden" name="revision" value={credential.revision} />
          <button type="submit" className="btn btn--danger btn--sm" disabled={deletePending}>
            {armed ? "Confirm — delete key" : "Delete"}
          </button>
        </form>
      </div>
      {armed ? (
        <p className="row__status row__status--error">
          Deletes the stored secret permanently.
          {lastKey ? " This is the only key — the provider cannot serve traffic without one." : ""}
        </p>
      ) : null}
      <StatusLine state={state.error || state.ok ? state : deleteState} />
    </li>
  );
}

/* ---------- Model capabilities on a provider ---------- */

const PATH_LABEL: Record<UpstreamApiPath, string> = {
  chat_completions: "chat",
  responses: "responses",
};

/* The primary way to declare models: ask the provider what it serves, then tick. A model whose ID
 * matches one Monet sells is one click; a provider-renamed model needs an explicit "serves" mapping
 * before its checkbox arms, so an unmappable capability can never be created by accident. */
export function ModelPicker({
  provider,
  sellableModels,
  declaredUpstreamIds,
}: {
  provider: UpstreamProvider;
  /** Model IDs Monet sells, from the rate card. */
  sellableModels: string[];
  /** upstream_model_ids already declared on this provider. */
  declaredUpstreamIds: string[];
}) {
  const [discoverState, discoverAction, discovering] = useActionState<ActionState, FormData>(discoverProviderModels, {});
  const [addState, addAction, adding] = useActionState<ActionState, FormData>(addDiscoveredModels, {});
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [mapping, setMapping] = useState<Record<string, string>>({});

  const declared = new Set(declaredUpstreamIds);
  const sellable = new Set(sellableModels);
  const models = discoverState.discoveredModels ?? [];

  const toggle = (id: string, on: boolean) =>
    setSelected((previous) => {
      const next = new Set(previous);
      if (on) next.add(id); else next.delete(id);
      return next;
    });

  if (models.length === 0) {
    return (
      <form action={discoverAction} className="stack stack--sm">
        <input type="hidden" name="provider_id" value={provider.id} />
        <div className="form-actions">
          <button type="submit" className="btn btn--primary btn--sm" data-state={discovering ? "loading" : undefined} disabled={discovering}>
            Load models from provider
          </button>
          <p className="field__help">
            Reads the provider&rsquo;s <code>/v1/models</code> with the stored key. Nothing is added
            until you choose.
          </p>
        </div>
        <StatusLine state={discoverState} />
      </form>
    );
  }

  const selectable = models.filter((id) => !declared.has(id));

  return (
    <form action={addAction} className="stack stack--sm">
      <input type="hidden" name="provider_id" value={provider.id} />
      <ul className="pick">
        {models.map((id) => {
          if (declared.has(id)) {
            return (
              <li key={id} className="pick__row">
                <span className="pick__name mono">{id}</span>
                <span className="chip chip--ok">declared</span>
              </li>
            );
          }
          if (sellable.has(id)) {
            return (
              <li key={id} className="pick__row">
                <label className="check">
                  <input
                    type="checkbox"
                    name={`model:${id}`}
                    checked={selected.has(id)}
                    onChange={(event) => toggle(id, event.target.checked)}
                    disabled={adding}
                  />
                  <span className="pick__name mono">{id}</span>
                </label>
              </li>
            );
          }
          const mapped = mapping[id] ?? "";
          return (
            <li key={id} className="pick__row pick__row--foreign">
              <label className="check">
                <input
                  type="checkbox"
                  name={`model:${id}`}
                  checked={selected.has(id)}
                  onChange={(event) => toggle(id, event.target.checked)}
                  disabled={adding || !mapped}
                />
                <span className="pick__name mono">{id}</span>
              </label>
              <label className="pick__map">
                <span className="meta">sold as</span>
                <span className="select select--mono">
                  <select
                    name={`map:${id}`}
                    className="input"
                    value={mapped}
                    disabled={adding}
                    onChange={(event) => {
                      setMapping((previous) => ({ ...previous, [id]: event.target.value }));
                      if (!event.target.value) toggle(id, false);
                    }}
                  >
                    <option value="">not sold</option>
                    {sellableModels.map((model) => (
                      <option key={model} value={model}>{model}</option>
                    ))}
                  </select>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
      {selectable.length === 0 ? (
        <p className="field__help">Every model the provider reports is already declared.</p>
      ) : (
        <div className="form-actions">
          <button
            type="submit"
            className="btn btn--primary btn--sm"
            data-state={adding ? "loading" : undefined}
            disabled={adding || selected.size === 0}
          >
            Add {selected.size > 0 ? `${selected.size} ` : ""}selected
          </button>
          <p className="field__help">
            Models Monet doesn&rsquo;t sell need a &ldquo;sold as&rdquo; mapping before they can be
            ticked. Both API paths are enabled.
          </p>
        </div>
      )}
      <StatusLine state={addState} />
    </form>
  );
}

export function AddCapabilityForm({ provider }: { provider: UpstreamProvider }) {
  const [state, action, pending] = useActionState<ActionState, FormData>(createUpstreamModel, {});
  const formRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    if (state.ok) formRef.current?.reset();
  }, [state]);

  return (
    <form ref={formRef} action={action} className="stack stack--sm">
      <input type="hidden" name="provider_id" value={provider.id} />
      <div className="grid-2">
        <div className="field">
          <label className="field__label" htmlFor={`c-canon-${provider.id}`}>Canonical model</label>
          <input id={`c-canon-${provider.id}`} name="canonical_model_id" className="input" placeholder="gpt-5.6-luna" required disabled={pending} />
          <p className="field__help">Monet&rsquo;s name for the model family.</p>
        </div>
        <div className="field">
          <label className="field__label" htmlFor={`c-up-${provider.id}`}>Provider&rsquo;s model ID</label>
          <input id={`c-up-${provider.id}`} name="upstream_model_id" className="input" placeholder="vendor-luna-2026" required disabled={pending} />
          <p className="field__help">What this provider calls it.</p>
        </div>
      </div>
      <fieldset className="checkset">
        <legend className="field__label">Supported API paths</legend>
        <label className="check">
          <input name="chat_completions" type="checkbox" defaultChecked disabled={pending} />
          <span>Chat Completions</span>
        </label>
        <label className="check">
          <input name="responses" type="checkbox" defaultChecked disabled={pending} />
          <span>Responses</span>
        </label>
      </fieldset>
      <div className="form-actions">
        <button type="submit" className="btn btn--primary btn--sm" data-state={pending ? "loading" : undefined} disabled={pending}>
          Add model
        </button>
      </div>
      <StatusLine state={state} />
    </form>
  );
}

export function CapabilityRow({
  capability,
  testableModelIds,
}: {
  capability: UpstreamModelCapability;
  /* Public models whose canonical model matches this capability. The gateway rejects a test against
   * any other, so offering the full rate card here would offer guaranteed failures. */
  testableModelIds: string[];
}) {
  const [testState, testAction, testPending] = useActionState<ActionState, FormData>(testUpstream, {});
  const [removeState, removeAction, removePending] = useActionState<ActionState, FormData>(deleteUpstreamModel, {});
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), 4000);
    return () => clearTimeout(timer);
  }, [armed]);

  const result = testState.error || testState.ok ? testState : removeState;
  const only = testableModelIds.length === 1 ? testableModelIds[0] : null;

  return (
    <li className="row">
      <div className="row__main">
        <span className="row__name mono">{capability.canonical_model_id}</span>
        <span className="row__meta meta">
          → {capability.upstream_model_id} · {capability.api_paths.map((path) => PATH_LABEL[path]).join(" + ")}
          {capability.verified_at ? " · tested" : ""}
        </span>
      </div>

      <div className="row__actions">
        {testableModelIds.length === 0 ? (
          <span className="field__help">No customer-facing model maps to this one yet, so it cannot be tested.</span>
        ) : (
          <form action={testAction} className="inline-form">
            <input type="hidden" name="id" value={capability.id} />
            {/* One choice needs no picker — the button names it. Several get a labelled select. */}
            {only ? (
              <input type="hidden" name="public_model_id" value={only} />
            ) : (
              <label className="testas">
                <span className="testas__label">as</span>
                <span className="select select--mono">
                  {/* Options carry only the part that differs from the canonical name already shown on
                    * this row ("low" / "medium"). Showing the full id truncates to `gpt-5.6-luna-…`,
                    * hiding the one segment the choice is actually about. The value stays the full id. */}
                  <select
                    name="public_model_id"
                    className="input input--model"
                    disabled={testPending}
                    aria-label={`Customer-facing model to test ${capability.canonical_model_id} as`}
                  >
                    {testableModelIds.map((model) => (
                      <option key={model} value={model}>
                        {model.startsWith(`${capability.canonical_model_id}-`)
                          ? model.slice(capability.canonical_model_id.length + 1)
                          : model}
                      </option>
                    ))}
                  </select>
                </span>
              </label>
            )}
            <button
              type="submit"
              className="btn btn--ghost btn--sm"
              disabled={testPending}
              data-state={testPending ? "loading" : undefined}
              aria-label={only ? `Test as ${only}` : undefined}
            >
              Test
            </button>
          </form>
        )}
        <form
          action={removeAction}
          onSubmit={(event) => {
            if (!armed) {
              event.preventDefault();
              setArmed(true);
            }
          }}
        >
          <input type="hidden" name="id" value={capability.id} />
          <input type="hidden" name="revision" value={capability.revision} />
          <button type="submit" className="btn btn--danger btn--sm" disabled={removePending}>
            {armed ? "Confirm" : "Remove"}
          </button>
        </form>
      </div>

      {/* Rendered only when there is something to say — a reserved slot on every row is what made this
        * list twice as tall as it needed to be. */}
      {result.error || result.ok ? (
        <p className={`row__status${result.error ? " row__status--error" : " row__status--ok"}`} aria-live="polite">
          {result.error ?? result.ok}
        </p>
      ) : null}
    </li>
  );
}

/* ---------- Route editor: ordered fallback chain with drag-and-drop ---------- */

export interface RouteCandidate {
  capabilityId: string;
  providerName: string;
  providerEnabled: boolean;
  transport: UpstreamTransport;
  upstreamModelId: string;
  apiPaths: UpstreamApiPath[];
  capabilityEnabled: boolean;
}

interface Row {
  capabilityId: string;
  /** True when this row shares its priority tier with the row above, splitting traffic by weight. */
  tied: boolean;
  weight: number;
}

/** Position in the list is the priority tier; tied rows collapse into the tier above. */
function tiersFor(rows: Row[]): number[] {
  let tier = 0;
  return rows.map((row, index) => {
    if (index === 0 || !row.tied) tier += 1;
    return tier;
  });
}

function normalise(rows: Row[]): Row[] {
  // The first row can never be tied to something above it.
  return rows.map((row, index) => (index === 0 && row.tied ? { ...row, tied: false } : row));
}

export function RouteEditor({
  publicModelId,
  revision,
  initial,
  candidates,
}: {
  publicModelId: string;
  revision: number;
  initial: Array<{ capabilityId: string; priority: number; weight: number }>;
  candidates: RouteCandidate[];
}) {
  const [state, action, pending] = useActionState<ActionState, FormData>(publishModelRoute, {});

  const initialRows = useMemo<Row[]>(() => {
    const ordered = [...initial].sort((left, right) => left.priority - right.priority);
    return ordered.map((target, index) => ({
      capabilityId: target.capabilityId,
      tied: index > 0 && target.priority === ordered[index - 1].priority,
      weight: target.weight,
    }));
  }, [initial]);

  const [rows, setRows] = useState<Row[]>(initialRows);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const [candidateQuery, setCandidateQuery] = useState("");

  const byId = useMemo(
    () => new Map(candidates.map((candidate) => [candidate.capabilityId, candidate])),
    [candidates],
  );
  const tiers = tiersFor(rows);
  const tierSizes = tiers.reduce<Record<number, number>>((sizes, tier) => {
    sizes[tier] = (sizes[tier] ?? 0) + 1;
    return sizes;
  }, {});
  const available = candidates.filter((candidate) => !rows.some((row) => row.capabilityId === candidate.capabilityId));
  const candidateNeedle = candidateQuery.trim().toLowerCase();
  const shownCandidates = candidateNeedle
    ? available.filter((candidate) =>
        candidate.providerName.toLowerCase().includes(candidateNeedle) ||
        candidate.upstreamModelId.toLowerCase().includes(candidateNeedle))
    : available;
  const dirty =
    rows.length !== initialRows.length ||
    rows.some((row, index) =>
      row.capabilityId !== initialRows[index]?.capabilityId ||
      row.tied !== initialRows[index]?.tied ||
      row.weight !== initialRows[index]?.weight);

  const label = (capabilityId: string) => byId.get(capabilityId)?.providerName ?? "Unknown provider";

  const move = (from: number, to: number) => {
    if (to < 0 || to >= rows.length || from === to) return;
    const next = [...rows];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    /* Reordering must never invent a traffic split. Landing an untied row directly above a tied one
     * would silently absorb it into the moved row's tier, so both ends of the seam are untied and the
     * operator re-opts into any split explicitly. */
    const brokeTie = next[to].tied || (next[to + 1]?.tied ?? false);
    next[to] = { ...next[to], tied: false };
    if (next[to + 1]?.tied) next[to + 1] = { ...next[to + 1], tied: false };
    setRows(normalise(next));
    setAnnouncement(
      `${label(moved.capabilityId)} moved to position ${to + 1} of ${next.length}.` +
        (brokeTie ? " Traffic split cleared around the move — re-check the box to restore it." : ""),
    );
  };

  const add = (capabilityId: string) => {
    setRows(normalise([...rows, { capabilityId, tied: false, weight: 1 }]));
    setAnnouncement(`${label(capabilityId)} added at position ${rows.length + 1}.`);
  };

  const remove = (index: number) => {
    const removed = rows[index];
    setRows(normalise(rows.filter((_, position) => position !== index)));
    setAnnouncement(`${label(removed.capabilityId)} removed from the route.`);
  };

  const patch = (index: number, change: Partial<Row>) => {
    setRows(normalise(rows.map((row, position) => (position === index ? { ...row, ...change } : row))));
  };

  const payload = JSON.stringify(
    rows.map((row, index) => ({
      capability_id: row.capabilityId,
      priority: tiers[index],
      weight: tierSizes[tiers[index]] > 1 ? row.weight : 1,
    })),
  );

  if (candidates.length === 0) {
    return (
      <div className="empty empty--inline">
        <p className="empty__title">No provider can serve this model yet.</p>
        <p className="empty__body">
          Add a model capability to a provider whose canonical model matches this one. Until a route is
          published, requests for <code>{publicModelId}</code> are rejected rather than sent to a
          different model.
        </p>
      </div>
    );
  }

  return (
    <form action={action} className="stack">
      <input type="hidden" name="public_model_id" value={publicModelId} />
      <input type="hidden" name="revision" value={revision} />
      <input type="hidden" name="targets" value={payload} />

      {rows.length === 0 ? (
        <p className="field__help">
          Nothing routed yet — add a provider below. <code>{publicModelId}</code> stays unavailable
          until this is published.
        </p>
      ) : (
        <ol className="chain">
          {rows.map((row, index) => {
            const candidate = byId.get(row.capabilityId);
            const tier = tiers[index];
            const shared = tierSizes[tier] > 1;
            const inactive = candidate ? !candidate.providerEnabled || !candidate.capabilityEnabled : true;
            return (
              <li
                key={row.capabilityId}
                className={[
                  "chain__row",
                  dragIndex === index ? "is-dragging" : "",
                  row.tied ? "is-tied" : "",
                  shared ? "is-grouped" : "",
                  rows[index + 1]?.tied ? "has-tied-below" : "",
                ].filter(Boolean).join(" ")}
                draggable={!pending}
                onDragStart={() => setDragIndex(index)}
                onDragEnd={() => setDragIndex(null)}
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => {
                  event.preventDefault();
                  if (dragIndex !== null) move(dragIndex, index);
                  setDragIndex(null);
                }}
              >
                <span className="chain__tier tnum" aria-hidden="true">{shared ? `${tier}=` : tier}</span>
                <span className="chain__grip" aria-hidden="true" title="Drag to reorder" />

                <span className="chain__body">
                  <span className="chain__name">
                    {candidate?.providerName ?? "Unknown provider"}
                    {candidate?.transport === "codex_cli" ? <span className="chip chip--muted">Codex</span> : null}
                    {inactive ? <span className="chip chip--error">disabled</span> : null}
                  </span>
                  <span className="chain__meta meta">
                    {candidate ? `${candidate.upstreamModelId} · ${candidate.apiPaths.map((path) => PATH_LABEL[path]).join(" + ")}` : row.capabilityId}
                  </span>
                </span>

                {shared ? (
                  <span className="chain__weight">
                    <label className="field__label" htmlFor={`w-${publicModelId}-${row.capabilityId}`}>Share</label>
                    <input
                      id={`w-${publicModelId}-${row.capabilityId}`}
                      className="input input--tiny tnum"
                      value={row.weight}
                      inputMode="numeric"
                      onChange={(event) => patch(index, { weight: Number(event.target.value.replace(/\D/g, "")) || 1 })}
                      disabled={pending}
                    />
                  </span>
                ) : null}

                <span className="chain__controls">
                  <button
                    type="button"
                    className="iconbtn"
                    onClick={() => move(index, index - 1)}
                    disabled={pending || index === 0}
                    aria-label={`Move ${label(row.capabilityId)} up`}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="iconbtn"
                    onClick={() => move(index, index + 1)}
                    disabled={pending || index === rows.length - 1}
                    aria-label={`Move ${label(row.capabilityId)} down`}
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    className="iconbtn iconbtn--danger"
                    onClick={() => remove(index)}
                    disabled={pending}
                    aria-label={`Remove ${label(row.capabilityId)} from the route`}
                  >
                    ×
                  </button>
                </span>

                {index > 0 ? (
                  <label className="chain__tie check">
                    <input
                      type="checkbox"
                      checked={row.tied}
                      onChange={(event) => patch(index, { tied: event.target.checked })}
                      disabled={pending}
                    />
                    <span>Split traffic with the row above instead of using it as a fallback</span>
                  </label>
                ) : null}
              </li>
            );
          })}
        </ol>
      )}

      <p className="sr-live" role="status" aria-live="polite">{announcement}</p>

      {available.length > 0 ? (
        <div className="addable">
          <p className="field__label">Available providers</p>
          {/* Past a handful of candidates a flat pill list stops being scannable. */}
          {available.length > 8 ? (
            <input
              className="input"
              type="search"
              value={candidateQuery}
              placeholder={`Filter ${available.length} providers`}
              onChange={(event) => setCandidateQuery(event.target.value)}
              aria-label="Filter available providers"
              disabled={pending}
            />
          ) : null}
          <ul className="addable__list">
            {shownCandidates.map((candidate) => (
              <li key={candidate.capabilityId}>
                <button type="button" className="addable__btn" onClick={() => add(candidate.capabilityId)} disabled={pending}>
                  <span className="addable__plus" aria-hidden="true">+</span>
                  {candidate.providerName}
                  <span className="meta">{candidate.upstreamModelId}</span>
                  {!candidate.providerEnabled || !candidate.capabilityEnabled ? (
                    <span className="chip chip--error">disabled</span>
                  ) : null}
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="form-actions">
        <button type="submit" className="btn btn--primary btn--sm" data-state={pending ? "loading" : undefined} disabled={pending || !dirty}>
          {dirty ? "Publish order" : "Published"}
        </button>
        <p className="field__help">
          Top row runs first. A tied row shares its tier and splits traffic by weight. Publishing
          replaces revision {revision} in one atomic swap.
        </p>
      </div>
      <StatusLine state={state} />
    </form>
  );
}
