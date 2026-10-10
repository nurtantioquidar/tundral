/* monet.sh is the studio's front door, not a product surface. A pixel-art pond with a small robot
 * painting at an easel says what Monet is; the instruments follow, each with its own pixel icon;
 * the page asks for one thing: an email. Product detail, pricing, and sign-in belong to each
 * product's own surface. Design law: DESIGN.md. */

import { PixelIcon } from "./icons";
import { PondScene } from "./pond";

interface Product {
  id: "butterfly" | "cocoon" | "nimble" | "inver" | "cicada";
  name: string;
  role: string;
  summary: string;
  shipsAs: string;
  /* Maturity as each product's own README states it. */
  stage: string;
  /* Only set once the product's public surface is served, so the page never links to a dead host. */
  href?: string;
}

/* In the order a request meets them. */
const PRODUCTS: Product[] = [
  {
    id: "butterfly",
    name: "Butterfly",
    role: "Coding harness",
    summary: "A coding agent built around the prompt cache. Keeps hits high, compacts rarely, checks its own work.",
    shipsAs: "Rust CLI",
    stage: "Alpha",
  },
  {
    id: "cocoon",
    name: "Cocoon",
    role: "Credential firewall",
    summary: "The agent holds a session token. The real key is added only after the request passes policy.",
    shipsAs: "Rust CLI",
    stage: "Prototype",
  },
  {
    id: "nimble",
    name: "Nimble",
    role: "Model router",
    summary: "One OpenAI-compatible endpoint that picks the model for each request and keeps caches warm.",
    shipsAs: "Local service",
    stage: "In development",
  },
  {
    id: "inver",
    name: "Inver",
    role: "LLM gateway",
    summary: "A prepaid gateway. Every workload gets its own key; every request settles to an exact cost.",
    shipsAs: "Hosted API",
    stage: "Pilot",
    href: process.env.NEXT_PUBLIC_INVER_URL || undefined,
  },
  {
    id: "cicada",
    name: "Cicada",
    role: "Token ledger",
    summary: "One command that reads what is left on every model API account you use, all at once.",
    shipsAs: "Rust CLI",
    stage: "Alpha",
  },
];

const MAILTO = "mailto:tio@monet.sh";

export default function HubPage() {
  return (
    <>
      <section className="hero">
        <header className="bar">
          <a className="bar__brand" href="/">
            <PixelIcon id="lily" className="bar__icon" />
            Monet
          </a>
          <nav className="bar__nav" aria-label="Sections">
            <a href="#instruments">Instruments</a>
            <a href="#canvas">The canvas</a>
          </nav>
        </header>

        <div className="hero__copy">
          <h1 className="hero__title">
            <span className="hero__line">Instruments for AI,</span>
            <span className="hero__line">then the art.</span>
          </h1>
          <p className="hero__lede">
            Monet is a small studio named for the painter. We build the tools between developers and models, and next,
            the art made with them.
          </p>
          <div className="hero__actions">
            <a className="pill pill--primary" href={MAILTO}>
              Write to the studio →
            </a>
            <a className="pill pill--quiet" href="#instruments">
              See the instruments
            </a>
          </div>
        </div>

        <PondScene />
      </section>

      <main>
        <section id="instruments" className="tools" aria-labelledby="tools-title">
          <h2 id="tools-title" className="section-title">
            Instruments
          </h2>
          <p className="section-lede">
            Five tools on the path of one model call, in the order a request meets them. The coral pixels in each icon
            are the one thing that tool decides or protects.
          </p>
          <ol className="tools__grid">
            {PRODUCTS.map((product) => (
              <li key={product.id} className="tool">
                <PixelIcon id={product.id} label={`${product.name} icon`} className="tool__icon" />
                <h3 className="tool__name">
                  {product.href ? <a href={product.href}>{product.name}</a> : product.name}
                </h3>
                <p className="tool__role">{product.role}</p>
                <p className="tool__summary">{product.summary}</p>
                <p className="tool__meta">
                  {product.shipsAs} · {product.stage}
                </p>
              </li>
            ))}
          </ol>
        </section>

        <section id="canvas" className="canvas" aria-labelledby="canvas-title">
          <h2 id="canvas-title" className="section-title">
            The canvas
          </h2>
          <p className="section-lede">
            The robot at the easel is the point. The instruments come first so the art can follow: work where the model
            holds the brush. Nothing is hung yet.
          </p>
          <a className="pill pill--primary" href={MAILTO}>
            Write to the studio →
          </a>
        </section>
      </main>
    </>
  );
}
