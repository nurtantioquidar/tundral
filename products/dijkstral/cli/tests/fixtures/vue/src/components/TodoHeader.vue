<script lang="ts">
export const HEADER_ID = 'todo-header'
</script>

<script setup lang="ts">
import capitalize, { pluralize } from '../utils/format'

const props = defineProps<{ count: number }>()

function label() {
  return capitalize(pluralize(props.count, 'task'))
}
</script>

<template>
  <h1 :id="HEADER_ID">{{ label() }}</h1>
</template>
