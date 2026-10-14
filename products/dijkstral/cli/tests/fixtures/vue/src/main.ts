import { createApp } from 'vue'
import App from './App.vue'
import { createStore } from './store'

// NOTE: one app instance per page
export function mount(selector: string) {
  const store = createStore()
  createApp(App).use(store).mount(selector)
}
