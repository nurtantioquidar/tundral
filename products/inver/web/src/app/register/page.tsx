import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { RegisterForm } from "@/components/SignInForm";
import { readCustomerSession } from "@/server/customer-session";

export const metadata: Metadata = { title: "Create account — Inver" };

export default async function RegisterPage() {
  if (await readCustomerSession()) redirect("/console");

  return (
    <>
      <header className="nav">
        <div className="container nav__inner">
          <Link className="nav__brand" href="/">
            Inver<span className="nav__brand-tick">.</span>
          </Link>
        </div>
      </header>
      <main className="auth container">
        <h1 className="auth__title">Create your account</h1>
        <p className="auth__sub">
          Registration creates your prepaid account and one default API key. The key is shown once, and you can add
          more keys with their own spending caps in the console.
        </p>
        <RegisterForm />
        <p className="auth__switch">
          Already registered? <Link href="/signin">Sign in</Link>
        </p>
      </main>
    </>
  );
}
