"use client";

import { useRouter } from "next/navigation";
import { startTransition } from "react";

/* Application error boundary — the floor under every route. Expected operational states (gateway
 * unavailable, runtime-mode mismatch) should be caught page-side and rendered as product notices;
 * anything that still reaches here gets a calm, styled page instead of a framework error screen.
 * The error message is shown, but gateway response bodies never reach this boundary because the
 * BFF only throws GatewayError messages, never raw payloads. */
export default function AppError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const router = useRouter();
  /* reset() alone replays the client render against cached server output; refreshing first re-runs
   * the server components, so a recovered gateway actually recovers the page. */
  const retry = () =>
    startTransition(() => {
      router.refresh();
      reset();
    });
  return (
    <main className="console-main container">
      <div className="console-head">
        <div>
          <h1 className="console-head__title">Something went wrong</h1>
          <p className="console-head__sub">
            The page hit an unexpected error. Reloading usually recovers it; if it persists, check
            that the inference gateway is running and reachable.
          </p>
        </div>
      </div>
      <div className="notice notice--error">
        {error.message || "Unexpected error"}
        {error.digest ? <span className="meta"> · ref {error.digest}</span> : null}
      </div>
      <div className="form-actions">
        <button type="button" className="btn btn--primary" onClick={retry}>
          Try again
        </button>
        <a className="btn btn--ghost" href="/">
          Back to the landing page
        </a>
      </div>
    </main>
  );
}
