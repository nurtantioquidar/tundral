<script context="module" lang="ts">
  export const CART_KEY = 'cart'
  export function emptyMessage(): string {
    return plural(0, 'item')
  }
</script>

<script lang="ts">
  import { cart, total, sumPrices } from './stores'
  import { money, plural } from './format.js'
  import CartRow from './CartRow.svelte'

  export let title = 'Cart'
  export let compact = false

  // WHY: recompute only when the cart changes
  $: subtotal = sumPrices($cart)

  function label() {
    return `${title}: ${money($total)}`
  }

  function clear() {
    cart.set([])
    announce()
  }

  function announce() {
    console.log(label())
  }
</script>

<section class:compact>
  <h2>{label()}</h2>
  {#each $cart as item (item.id)}
    <CartRow {item} />
  {:else}
    <p>{emptyMessage()}</p>
  {/each}
  <button on:click={clear}>Clear</button>
</section>

<style>
  .compact { font-size: 0.8rem; }
</style>
