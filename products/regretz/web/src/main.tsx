/* The Effect boundary. Everything below this file is plain React over plain data: the ledger is
 * decoded, checked and scored here, once, and what reaches a component is an array of objects. */

import { Effect, Either } from "effect";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import entriesJson from "./data/entries.json";
import pricesJson from "./data/prices.json";
import { Register } from "./register/Register";
import { formatDay } from "./register/format";
import { loadLedger } from "./scoring/ledger";
import "./styles/base.css";
import "./register/register.css";

/** Today at UTC midnight. Days-held for an open decision is a whole number of days, so the clock's
 * time of day must not leak into it. */
const startOfToday = (): Date => {
  const now = new Date();
  return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
};

const result = Effect.runSync(Effect.either(loadLedger(entriesJson, pricesJson, startOfToday())));

/** What happened, and what to do about it. No apology, no prefix. */
const explain = (error: Extract<typeof result, { _tag: "Left" }>["left"]): string => {
  switch (error._tag) {
    case "MissingPriceData":
      return `${error.ticker} has no close on ${formatDay(error.date)}. Add that day to prices.json.`;
    case "BenchmarkGap":
      return `IHSG has no close on ${formatDay(error.date)}. Every entry scored across that day is unscoreable until it is added to prices.json.`;
    case "DateOutsideSeries":
      return `${error.ticker} is priced from ${formatDay(error.seriesStart)} to ${formatDay(error.seriesEnd)}, which does not cover ${formatDay(error.date)}. Extend the series.`;
    case "UnresolvedEntry":
      return `${error.entryId} is still open and cannot be scored. Resolve it or leave it out of the total.`;
    case "DecisionDensityExceeded":
      return `Week ${error.isoWeek} holds ${error.count} decisions against a cap of ${error.cap}. Logging passes faster than they can be researched is how a ledger gets gamed.`;
    default:
      return "entries.json or prices.json does not match the ledger schema. Check the failing field and reload.";
  }
};

const root = document.getElementById("root");
if (root === null) throw new Error("index.html is missing its #root element");

createRoot(root).render(
  <StrictMode>
    {Either.isRight(result) ? (
      <Register ledger={result.right} />
    ) : (
      <main className="register">
        <p className="register__wordmark label">regretz</p>
        <p className="register__standfirst">{explain(result.left)}</p>
      </main>
    )}
  </StrictMode>,
);
