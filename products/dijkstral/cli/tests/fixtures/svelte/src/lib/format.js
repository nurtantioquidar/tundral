// NOTE: prices are integer cents
export function money(cents) {
  return `$${(cents / 100).toFixed(2)}`
}

export function plural(n, word) {
  return n === 1 ? word : word + 's'
}
