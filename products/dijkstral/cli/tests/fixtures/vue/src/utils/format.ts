import type { Todo } from '../types'

export function formatTitle(todo: Todo): string {
  return todo.done ? `[x] ${todo.title}` : `[ ] ${todo.title}`
}

export function pluralize(n: number, word: string) {
  return n === 1 ? word : `${word}s`
}

export default function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1)
}
