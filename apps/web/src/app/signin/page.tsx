import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { SignInForm } from "@/components/SignInForm";
import { readCustomerSession } from "@/server/customer-session";

export const metadata: Metadata = { title: "Sign in — Monet" };

export default async function SignInPage() {
  if (await readCustomerSession()) redirect("/console");

  return (
    <>
      <header className="nav">
        <div className="container nav__inner">
          <Link className="nav__brand" href="/">
            Monet<span className="nav__brand-tick">.</span>
          </Link>
        </div>
      </header>
      <main className="auth container">
        <h1 className="auth__title">Sign in to Monet</h1>
        <p className="auth__sub">
          Use your account email and password. API keys stay separate so you can revoke or limit them without losing
          access to your console.
        </p>
        <SignInForm />
        <p className="auth__switch">
          New to Monet? <Link href="/register">Create an account</Link>
        </p>
      </main>
    </>
  );
}
