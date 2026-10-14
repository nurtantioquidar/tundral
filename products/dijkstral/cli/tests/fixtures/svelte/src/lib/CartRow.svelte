<script lang="ts">
  import type { Item } from './stores'
  import { money } from './format.js'

  export let item: Item

  function price() {
    return money(item.price)
  }
</script>

<li>{item.name} - {price()}</li>
