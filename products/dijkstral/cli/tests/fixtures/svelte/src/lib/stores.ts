import { writable, derived } from 'svelte/store'

export interface Item {
  id: number
  name: string
  price: number
}

export const cart = writable<Item[]>([])

export const total = derived(cart, ($cart) => sumPrices($cart))

export function sumPrices(items: Item[]): number {
  return items.reduce((n, i) => n + i.price, 0)
}

export function addItem(item: Item) {
  cart.update((items) => [...items, item])
}
