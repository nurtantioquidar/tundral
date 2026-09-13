/* The register: one scrolling ledger, a header rule of column labels, and a footer that totals
 * alpha forgone. No cards, no metric grid across the top, no centred hero — the first thing on the
 * page after the wordmark is the ledger itself. */

import type { Ledger } from "../scoring/ledger";
import { formatAlphaPp, formatCount } from "./format";
import { RegisterRow } from "./RegisterRow";

export function Register({ ledger }: { ledger: Ledger }) {
  return (
    <main className="register">
      {/* The ink-bleed the stamp is filtered through. Displacement, not a shadow or a gradient. */}
      <svg className="register__defs" aria-hidden="true" focusable="false">
        <filter id="ink-bleed" x="-10%" y="-25%" width="120%" height="150%">
          <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="7" result="noise" />
          <feDisplacementMap in="SourceGraphic" in2="noise" scale="1.4" xChannelSelector="R" yChannelSelector="G" />
        </filter>
      </svg>

      <header className="register__head">
        <p className="register__wordmark label">regretz</p>
        <h1 className="register__title">A ledger of decisions not taken.</h1>
        <p className="register__standfirst">
          Every figure is measured against the IHSG over the same window. A stock that rose while the
          index rose further is a correct pass.
        </p>
      </header>

      {ledger.rows.length === 0 ? (
        <p className="register__empty">
          The first entry is the one you talked yourself out of this week. Write down why.
        </p>
      ) : (
        <>
          <div className="row row--head" role="presentation">
            <span className="label">date</span>
            <span className="label">decision</span>
            <span className="label row__alpha">vs ihsg</span>
            <span className="label row__days">days held</span>
          </div>

          <ol className="register__rows">
            {ledger.rows.map((row) => (
              <RegisterRow key={row.entry.id} row={row} />
            ))}
          </ol>

          <div className="row row--total">
            <span className="label">total</span>
            <p className="register__totalNote">
              Alpha forgone across {formatCount(ledger.resolvedCount)} resolved decisions, IHSG-adjusted.{" "}
              {formatCount(ledger.openCount)} still open and not counted.
            </p>
            <span className="row__alpha mono register__totalFigure">
              {formatAlphaPp(ledger.totalAlphaPp)}
              <span className="row__unit"> pp</span>
            </span>
            <span className="row__days" />
          </div>
        </>
      )}
    </main>
  );
}
