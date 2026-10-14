<template>
  <ul>
    <TodoItem v-for="t in todos" :key="t.id" :todo="t" @click="toggle(t.id)" />
  </ul>
</template>

<script lang="ts">
import { defineComponent, PropType } from 'vue'
import TodoItem from './TodoItem.vue'
import { formatTitle } from '../utils/format'
import type { Todo } from '../types'

export default defineComponent({
  name: 'TodoList',
  components: { TodoItem },
  props: {
    todos: { type: Array as PropType<Todo[]>, required: true },
  },
  emits: ['toggle'],
  methods: {
    toggle(id: number) {
      this.$emit('toggle', id)
    },
    titles() {
      return this.todos.map((t) => formatTitle(t))
    },
  },
})
</script>
