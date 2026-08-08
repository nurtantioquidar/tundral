/* monet.sh is the product-line index, not a marketing site. Its only job is to say what Monet is
 * and route a visitor to the right product subdomain in one click. Product detail, pricing, and
 * sign-in belong to each product's own surface — never duplicated here, where they would drift. */

type ProductStatus = "live" | "building";

interface Product {
  name: string;
  host: string;
  href: string;
  summary: string;
  status: ProductStatus;
}

/* Hosts are configurable so a local checkout can point at the dev ports (3001/3002) without
 * editing source. Production defaults are the real subdomains. */
const PRODUCTS: Product[] = [
  {
    name: "Inver",
    host: "inver.monet.sh",
    href: process.env.NEXT_PUBLIC_INVER_URL || "https://inver.monet.sh",
    summary:
      "A prepaid, OpenAI-compatible inference gateway. Point any coding harness at one endpoint, give each workload its own key, and read the settled cost of every request.",
    status: "live",
  },
  {
    name: "Regretz",
    host: "regretz.monet.sh",
    href: process.env.NEXT_PUBLIC_REGRETZ_URL || "https://regretz.monet.sh",
    summary:
      "An anti-portfolio: the things that were built and did not work, and what each one actually cost to learn.",
    status: "building",
  },
];

const STATUS_LABEL: Record<ProductStatus, string> = {
  live: "Live",
  building: "In progress",
};

export default function HubPage() {
  return (
    <main className="shell">
      <header className="masthead">
        <p className="wordmark">Monet</p>
        <h1 className="statement">Small products that meter what they cost.</h1>
        <p className="lede">
          Each product runs on its own subdomain with its own account and its own balance. Nothing here is a
          suite — pick the one you came for.
        </p>
      </header>

      {/* role="list" keeps list semantics in Safari, which drops them when list-style is none. */}
      <ul className="products" role="list">
        {PRODUCTS.map((product) => (
          <li key={product.name}>
            <a className="product" href={product.href}>
              <span className="product__head">
                <span className="product__name">{product.name}</span>
                <span className={`badge badge--${product.status}`}>{STATUS_LABEL[product.status]}</span>
              </span>
              <span className="product__summary">{product.summary}</span>
              <span className="product__host" aria-hidden="true">
                {product.host}
                <span className="product__arrow">→</span>
              </span>
            </a>
          </li>
        ))}
      </ul>

      <footer className="foot">
        <span>Monet</span>
        <span className="foot__host">monet.sh</span>
      </footer>
    </main>
  );
}
