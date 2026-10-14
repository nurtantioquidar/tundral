<script setup lang="ts">
import { computed } from 'vue'
import { formatTitle } from '../utils/format'
import type { Todo } from '../types'

interface Props {
  todo: Todo
}

const props = defineProps<Props>()
const text = computed(() => formatTitle(props.todo))

export function isDone(todo: Todo) {
  return todo.done
}
</script>

<template>
  <li :class="{ done: isDone(todo) }">{{ text }}</li>
</template>
