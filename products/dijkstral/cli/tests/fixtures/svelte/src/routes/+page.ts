import type { Item } from '../lib/stores'
import { sumPrices } from '../lib/stores'

export function load() {
  const products: Item[] = [{ id: 1, name: 'Tea', price: 350 }]
  return { title: 'Shop', products, total: sumPrices(products) }
}
