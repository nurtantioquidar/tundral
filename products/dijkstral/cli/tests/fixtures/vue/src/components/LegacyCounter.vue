<template>
  <!-- <script>function commentedOut() {}</script> -->
  <button @click="increment">{{ count }}</button>
</template>

<script>
import { pluralize } from '../utils/format'

export default {
  data() {
    return { count: 0 }
  },
  methods: {
    increment() {
      this.count++
      return pluralize(this.count, 'click')
    },
  },
}
</script>
