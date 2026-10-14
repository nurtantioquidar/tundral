import { ref, computed } from 'vue'
import type { Todo, Filter } from '../types'

export function useFilter(todos: () => Todo[]) {
  const filter = ref<Filter>('all')
  const visible = computed(() => applyFilter(todos(), filter.value))
  return { filter, visible }
}

function applyFilter(list: Todo[], f: Filter) {
  if (f === 'all') return list
  return list.filter((t) => (f === 'done' ? t.done : !t.done))
}
