/* Placeholder surface for regretz.monet.sh. It exists so the product-line routing is provable end to
 * end before the anti-portfolio's content model is designed. Do not grow features here — replace
 * this page once the entry shape (what was built, what it cost, what it taught) is decided. */

const HUB_URL = process.env.NEXT_PUBLIC_HUB_URL || "https://monet.sh";

export default function RegretzPlaceholderPage() {
  return (
    <main className="shell">
      <header>
        <p className="wordmark">Regretz</p>
        <h1 className="statement">An anti-portfolio.</h1>
        <p className="lede">
          The things that were built and did not work, and what each one actually cost to learn. Nothing is
          published yet.
        </p>
      </header>

      <footer className="foot">
        <a className="foot__back" href={HUB_URL}>
          <span aria-hidden="true">←</span> monet.sh
        </a>
        <span className="foot__host">regretz.monet.sh</span>
      </footer>
    </main>
  );
}
