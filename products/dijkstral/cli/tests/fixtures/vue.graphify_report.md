# Graph Report - vue  (2026-10-29)

## Corpus Check
- 12 files · ~614 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 44 nodes · 84 edges · 6 communities (5 shown, 1 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- format.ts
- Todo
- TodoItem.vue
- format.spec.ts
- App.vue
- useFilter.ts

## God Nodes (most connected - your core abstractions)
1. `Todo` - 9 edges
2. `TodoStore` - 5 edges
3. `formatTitle()` - 5 edges
4. `pluralize()` - 5 edges
5. `createStore()` - 4 edges
6. `label()` - 3 edges
7. `isDone()` - 3 edges
8. `useFilter()` - 3 edges
9. `capitalize()` - 3 edges
10. `onToggle()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `increment()` --calls--> `pluralize()`  [EXTRACTED]
  src/components/LegacyCounter.vue → src/utils/format.ts
- `Props` --references--> `Todo`  [EXTRACTED]
  src/components/TodoItem.vue → src/types.ts
- `text` --calls--> `formatTitle()`  [EXTRACTED]
  src/components/TodoItem.vue → src/utils/format.ts
- `titles()` --calls--> `formatTitle()`  [EXTRACTED]
  src/components/TodoList.vue → src/utils/format.ts
- `StoreState` --references--> `Todo`  [EXTRACTED]
  src/store.ts → src/types.ts

## Import Cycles
- None detected.

## Communities (6 total, 1 thin omitted)

### Community 0 - "format.ts"
Cohesion: 0.29
Nodes (6): increment(), HEADER_ID, label(), props, capitalize(), pluralize()

### Community 1 - "Todo"
Cohesion: 0.27
Nodes (6): Props, mount(), createStore(), StoreState, TodoStore, Todo

### Community 2 - "TodoItem.vue"
Cohesion: 0.48
Nodes (3): text, titles(), formatTitle()

### Community 4 - "App.vue"
Cohesion: 0.50
Nodes (3): logToggle(), onToggle(), { visible }

### Community 5 - "useFilter.ts"
Cohesion: 0.60
Nodes (3): applyFilter(), useFilter(), Filter

## Knowledge Gaps
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Todo` connect `Todo` to `format.ts`, `TodoItem.vue`, `useFilter.ts`?**
  _High betweenness centrality (0.115) - this node is a cross-community bridge._
- **Why does `TodoStore` connect `Todo` to `App.vue`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._