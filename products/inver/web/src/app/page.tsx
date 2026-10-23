import Link from "next/link";
import type { PricedModel } from "@tundral/contracts";
import { CopyButton } from "@/components/CopyButton";
import { SiteNav } from "@/components/SiteNav";
import { rateUsd } from "@/lib/format";
import { GatewayError, gatewayCustomer } from "@/server/gateway-client";

const BASE_URL_EXAMPLE = process.env.NEXT_PUBLIC_GATEWAY_URL || "https://gateway.inver.example/v1";

/* The landing page reads the live rate card so public pricing can never drift from what the console
 * and the ledger charge. Pricing is not essential to this page, so every failure mode degrades to an
 * honest empty state — never a stale hardcoded price list and never a framework error page.
 *
 * The catch is deliberately broad: an unreachable gateway rejects with a TypeError from fetch, not a
 * GatewayError, and a misconfigured deployment throws before the request is made. Narrowing this to
 * GatewayError would send the most likely outage straight to the error overlay. */
async function readRateCard(): Promise<{ version: string; models: PricedModel[] } | null> {
  try {
    const pricing = await gatewayCustomer.pricing();
    return { version: pricing.rate_card_version, models: pricing.data };
  } catch (cause) {
    const reason = cause instanceof GatewayError ? `gateway ${cause.status}` : "gateway unreachable";
    console.warn(`[landing] rate card unavailable (${reason}) — rendering the pricing empty state`);
    return null;
  }
}

const STAGES = [
  { id: "stage-1", num: "1.0", label: "Create account" },
  { id: "stage-2", num: "2.0", label: "Add credit" },
  { id: "stage-3", num: "3.0", label: "Connect client" },
  { id: "pricing", num: "4.0", label: "Watch it settle" },
];

export default async function LandingPage() {
  const rateCard = await readRateCard();

  return (
    <>
      <SiteNav />
      <main>
        <section className="hero container">
          <div className="hero__grid">
            <div>
              <h1 className="hero__title">Model access with one clear balance.</h1>
              <p className="hero__lede">
                Inver is an OpenAI-compatible endpoint on top of a prepaid balance. Point any coding harness at it,
                give each workload its own key, and read the settled cost of every request in your console.
              </p>
              <div className="hero__actions">
                <Link className="btn btn--primary btn--pill" href="/register">
                  Create account
                </Link>
                <Link className="link-cta" href="/signin">
                  Sign in
                </Link>
              </div>
            </div>
            {/* role="list" keeps list semantics in Safari, which drops them when list-style is none. */}
            <ol className="hero__index" role="list">
              {STAGES.map((stage) => (
                <li key={stage.id}>
                  <a href={`#${stage.id}`}>
                    <span className="hero__index-num">{stage.num}</span>
                    {stage.label}
                  </a>
                </li>
              ))}
            </ol>
          </div>
        </section>

        <section className="section section--spine container">
          <ol className="stages" role="list">
            <li className="stage" id="stage-1">
              <p className="stage__num">1.0</p>
              <h2 className="stage__title">Create your account.</h2>
              <p className="stage__lede">
                Register with an email and a password. Inver issues your first API key in the same step and shows the
                secret <strong>once</strong> — copy it before you leave the page. You can create more keys, rename
                them, or revoke them from the console at any time.
              </p>
            </li>

            <li className="stage" id="stage-2">
              <p className="stage__num">2.0</p>
              <h2 className="stage__title">Add prepaid credit.</h2>
              <p className="stage__lede">
                Credit lands in <strong>one shared account balance</strong>. Every key you create spends from that same
                balance, so there is only ever one number to watch.
              </p>
              <p className="stage__lede">
                Give a key a lifetime spending cap when you want a guardrail — a cap limits how much that key may spend
                against the shared balance. It is not a separate wallet and nothing is transferred into it.
              </p>
            </li>

            <li className="stage" id="stage-3">
              <p className="stage__num">3.0</p>
              <h2 className="stage__title">Connect your client.</h2>
              <p className="stage__lede">
                Set two environment variables. Anything that speaks Chat Completions or Responses works without a
                client library change.
              </p>
              <figure className="snippet stage__aside">
                <figcaption className="snippet__cap">
                  Environment
                  <CopyButton value={BASE_URL_EXAMPLE} label="Copy base URL" />
                </figcaption>
                <pre className="snippet__pre">
                  <code>
                    <span className="tok-cmd">export</span> OPENAI_BASE_URL={BASE_URL_EXAMPLE}
                    {"\n"}
                    <span className="tok-cmd">export</span> OPENAI_API_KEY=sk-cg-…
                  </code>
                </pre>
              </figure>
            </li>

            <li className="stage" id="pricing">
              <p className="stage__num">4.0</p>
              <h2 className="stage__title">Watch every request settle.</h2>
              <p className="stage__lede">
                Each request reserves against your balance, then settles at the published rate once the tokens are
                counted. Your console attributes every settled request to a model and a key, so the cost you read is
                the cost you were charged.
              </p>

              <div className="stage__aside">
                <h3 className="section__title">Current model rates</h3>
                <p className="section__note">
                  USD per one million tokens. Cached input is billed at the lower cache-read rate.
                </p>

                {rateCard === null ? (
                  <div className="empty">
                    <p className="empty__title">Rates are momentarily unavailable.</p>
                    <p className="empty__body">
                      The gateway did not answer in time. Reload in a moment, or{" "}
                      <Link className="link-cta" href="/signin">
                        sign in
                      </Link>{" "}
                      to read the live rate card from your console.
                    </p>
                  </div>
                ) : (
                  <>
                    <div className="ratecard-wrap">
                      <table className="ratecard tnum">
                        <thead>
                          <tr>
                            <th scope="col">Model</th>
                            <th scope="col">Input</th>
                            <th scope="col">Cached input</th>
                            <th scope="col">Output</th>
                          </tr>
                        </thead>
                        <tbody>
                          {rateCard.models.map((model) => (
                            <tr key={model.id}>
                              <td className="ratecard__model">
                                {model.id}
                                {model.reasoning_effort ? (
                                  <span className="ratecard__effort">reasoning · {model.reasoning_effort}</span>
                                ) : null}
                              </td>
                              <td className="ratecard__price">{rateUsd(model.pricing.input)}</td>
                              <td className="ratecard__price">{rateUsd(model.pricing.cache_read)}</td>
                              <td className="ratecard__price">{rateUsd(model.pricing.output)}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                    <p className="ratecard__footnote">
                      Rate card <code>{rateCard.version}</code>. Settled usage keeps the rate-card version it was
                      priced against — published changes never reprice history.
                    </p>
                  </>
                )}
              </div>
            </li>
          </ol>

          <div className="stage__foot">
            <Link className="btn btn--primary btn--pill" href="/register">
              Create your account
            </Link>
          </div>
        </section>
      </main>

      <footer className="foot-stmt container">
        <p className="foot-stmt__line">One balance. Every key accounted for.</p>
        <div className="foot-stmt__meta">
          <span className="foot-stmt__brand">Inver</span>
          {/* The operator console is deliberately unlinked from public surfaces; operators reach
            * /admin directly. Advertising the admin entry point invites password probing. */}
          <div className="foot-stmt__links">
            <Link href="/signin">Sign in</Link>
          </div>
        </div>
      </footer>
    </>
  );
}
