<script>
  import Cart, { CART_KEY } from '$lib/Cart.svelte'
  import Badge from '$lib/Badge.svelte'
  import { addItem } from '../lib/stores'

  export let data

  function buy(item) {
    addItem(item)
  }
</script>

<Badge>{CART_KEY}</Badge>
<Cart title={data.title} />
{#each data.products as p}
  <button on:click={() => buy(p)}>{p.name}</button>
{/each}
