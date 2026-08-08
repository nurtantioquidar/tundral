import Link from "next/link";

/* Slim bordered bar for console surfaces — same vocabulary as the landing nav, fewer destinations. */
export function ConsoleShell({
  area,
  links,
  actions,
  children,
}: {
  area: string;
  links?: Array<{ href: string; label: string; current?: boolean }>;
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <>
      <header className="nav">
        <div className="container nav__inner">
          <Link className="nav__brand" href="/">
            Monet<span className="nav__brand-tick">.</span>
          </Link>
          <span className="meta">{area}</span>
          {links ? (
            <nav className="nav__links nav__links--console" aria-label="Console">
              {links.map((link) => (
                <Link
                  key={link.href}
                  className={`nav__link${link.current ? " nav__link--current" : ""}`}
                  href={link.href}
                  aria-current={link.current ? "page" : undefined}
                >
                  {link.label}
                </Link>
              ))}
            </nav>
          ) : null}
          <div className="nav__spacer" />
          {actions}
        </div>
      </header>
      <main className="console-main container">{children}</main>
    </>
  );
}
