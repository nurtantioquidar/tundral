import { reactive } from 'vue'
import type { Todo } from './types'

export interface StoreState {
  todos: Todo[]
}

export class TodoStore {
  state: StoreState = reactive({ todos: [] })

  add(title: string): Todo {
    const todo = { id: this.state.todos.length + 1, title, done: false }
    this.state.todos.push(todo)
    return todo
  }

  toggle(id: number) {
    const t = this.state.todos.find((x) => x.id === id)
    if (t) t.done = !t.done
  }
}

export function createStore() {
  return new TodoStore()
}
