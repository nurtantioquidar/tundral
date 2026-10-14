---
type: community
members: 5
---

# Community 1

**Members:** 5 nodes

## Members
- [[Format()]] - code - store/helpers.go
- [[N]] - code
- [[NewUserService()]] - code - service/service.go
- [[Sum()]] - code - store/store.go
- [[main()]] - code - cmd/app/main.go

## Live Query (requires Dataview plugin)

```dataview
TABLE source_file, type FROM #community/Community_1
SORT file.name ASC
```

## Connections to other communities
- 2 edges to [[_COMMUNITY_Community 0]]
- 1 edge to [[_COMMUNITY_Community 4]]
- 1 edge to [[_COMMUNITY_Community 3]]
- 1 edge to [[_COMMUNITY_Community 2]]
- 1 edge to [[_COMMUNITY_Community 5]]

## Top bridge nodes
- [[NewUserService()]] - degree 4, connects to 3 communities
- [[main()]] - degree 4, connects to 1 community
- [[Sum()]] - degree 3, connects to 1 community
- [[Format()]] - degree 2, connects to 1 community