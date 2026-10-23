import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { AdminSignInForm } from "@/features/admin/AdminForms";
import { hasAdminSession } from "@/server/admin-session";

export const metadata: Metadata = { title: "Operator sign in — Inver" };

export default async function AdminSignInPage() {
  if (await hasAdminSession()) redirect("/admin");

  return (
    <>
      <header className="nav">
        <div className="container nav__inner">
          <Link className="nav__brand" href="/">
            Inver<span className="nav__brand-tick">.</span>
          </Link>
          <span className="meta">Operator</span>
        </div>
      </header>
      <main className="auth container">
        <h1 className="auth__title">Operator console</h1>
        <p className="auth__sub">
          Customers, balances, credits, keys, and usage. The gateway admin key never leaves the server.
        </p>
        <AdminSignInForm />
      </main>
    </>
  );
}
