/* One line of the ledger. Four columns: date, decision, vs IHSG, days held.
 *
 * The vs-IHSG cell carries the row's whole verdict. A regret gets the stamp — violet, rotated,
 * serial and figure. A correct pass gets the same figure in plain ink and nothing else, because
 * being right about not acting is unremarkable and the visual language should say so. An open
 * decision gets the word, not a spinner. */

import type { LedgerRow } from "../scoring/ledger";
import { formatAlphaPp, formatCount, formatDay, formatRupiah } from "./format";

export function RegisterRow({ row }: { row: LedgerRow }) {
  const { entry } = row;

  return (
    <li className="row">
      <span className="row__date mono">{formatDay(entry.decidedOn)}</span>

      <div className="row__decision">
        <p className="row__thesis">{entry.thesis}</p>
        <p className="row__note">{entry.note}</p>
        <p className="row__meta label">
          {entry.id} · {entry.ticker} · {formatRupiah(entry.priceAtDecision)}
        </p>
      </div>

      <div className="row__alpha">
        {row._tag === "Open" ? (
          <span className="row__open label">unresolved</span>
        ) : row.alpha.isRegret ? (
          <span className="stamp mono">
            <span className="stamp__serial">{entry.id}</span>
            <span className="stamp__figure">
              {formatAlphaPp(row.alpha.alphaPp)}
              <span className="stamp__unit"> pp</span>
            </span>
          </span>
        ) : (
          <span className="row__figure mono">
            {formatAlphaPp(row.alpha.alphaPp)}
            <span className="row__unit"> pp</span>
          </span>
        )}
      </div>

      <span className="row__days mono">
        {formatCount(row._tag === "Open" ? row.daysHeld : row.alpha.daysHeld)}
      </span>
    </li>
  );
}
