<template>
  <main>
    <TodoHeader :count="visible.length" />
    <TodoList :todos="visible" @toggle="onToggle" />
    <EmptyState v-if="!visible.length" />
  </main>
</template>

<script setup lang="ts">
import { inject } from 'vue'
import TodoHeader from './components/TodoHeader.vue'
import TodoList from './components/TodoList.vue'
import EmptyState from './components/EmptyState.vue'
import { useFilter } from './composables/useFilter'
import { TodoStore } from './store'

const store = inject<TodoStore>('store')!
const { visible } = useFilter(() => store.state.todos)

// WHY: the list only emits ids, the store owns the data
function onToggle(id: number) {
  store.toggle(id)
  logToggle(id)
}

function logToggle(id: number) {
  console.debug('toggled', id)
}
</script>

<style scoped>
main { max-width: 40rem; }
</style>
