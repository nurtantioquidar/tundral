import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import TodoHeader from '../components/TodoHeader.vue'
import { isDone } from '../components/TodoItem.vue'

describe('header', () => {
  it('renders', () => {
    expect(mount(TodoHeader, { props: { count: 2 } }).text()).toContain('Tasks')
  })
})

export function sample() {
  return isDone({ id: 1, title: 'x', done: true })
}
