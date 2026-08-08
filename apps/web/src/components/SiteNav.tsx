import Link from "next/link";

/* N5 floating pill — content-sized and visibly detached, for public surfaces only. Console and
 * operator surfaces use the bordered bar in ConsoleShell. Below 40rem the link row drops and the
 * pill keeps the wordmark plus the primary action; sign-in stays in the hero actions. */
export function SiteNav() {
  return (
    <nav className="navpill" aria-label="Primary">
      <Link className="navpill__brand" href="/">
        Monet<span className="navpill__tick">.</span>
      </Link>
      <div className="navpill__links">
        <a className="navpill__link" href="/#pricing">
          Pricing
        </a>
        <Link className="navpill__link" href="/signin">
          Sign in
        </Link>
      </div>
      <Link className="btn btn--primary btn--pill" href="/register">
        Create account
      </Link>
    </nav>
  );
}
