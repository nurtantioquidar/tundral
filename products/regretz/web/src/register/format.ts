/* Indonesian formatting, all of it delegated to Intl. Nothing here builds a number by hand: the
 * difference between a tool built for BEI and a US template with tickers swapped in is that this
 * one gets `1.240` and `+218,4` right without anyone thinking about it. */

const alphaPp = new Intl.NumberFormat("id-ID", {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
  signDisplay: "always",
});

const rupiah = new Intl.NumberFormat("id-ID", {
  style: "currency",
  currency: "IDR",
  minimumFractionDigits: 0,
  maximumFractionDigits: 0,
});

const wholeNumber = new Intl.NumberFormat("id-ID", { maximumFractionDigits: 0 });

/* UTC, because every date in the ledger is stored at UTC midnight. Formatting in the reader's zone
 * would shift a decision by a day west of Greenwich. */
const day = new Intl.DateTimeFormat("id-ID", {
  day: "2-digit",
  month: "short",
  year: "2-digit",
  timeZone: "UTC",
});

/** `+149,88` / `-33,98`. The unit is rendered separately so the decimals stay aligned. */
export const formatAlphaPp = (pp: number): string => alphaPp.format(pp);

/** `Rp9.800`.
 *
 * Built from parts rather than from `format()` because ICU disagrees with itself about the gap
 * after the symbol: Chrome emits `Rp 9.800`, other builds emit `Rp9.800`. Dropping the literal
 * separator keeps the grouping, the symbol and the digits entirely Intl's business while making the
 * output stable across engines. */
export const formatRupiah = (amount: number): string =>
  rupiah
    .formatToParts(amount)
    .filter((part) => part.type !== "literal")
    .map((part) => part.value)
    .join("");

/** `04 Feb 26` */
export const formatDay = (date: Date): string => day.format(date);

/** `178` */
export const formatCount = (value: number): string => wholeNumber.format(value);
