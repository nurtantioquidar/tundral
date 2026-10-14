# Community 1

> 5 nodes

## Key Concepts

- **main()** (4 connections) — `cmd/app/main.go`
- **NewUserService()** (4 connections) — `service/service.go`
- **Sum()** (3 connections) — `store/store.go`
- **Format()** (2 connections) — `store/helpers.go`
- **N** (1 connections)

## Relationships

- [Community 0](Community_0.md) (2 shared connections)
- [Community 4](Community_4.md) (1 shared connections)
- [Community 3](Community_3.md) (1 shared connections)
- [Community 2](Community_2.md) (1 shared connections)
- [Community 5](Community_5.md) (1 shared connections)

## Source Files

- `cmd/app/main.go`
- `service/service.go`
- `store/helpers.go`
- `store/store.go`

## Audit Trail

- EXTRACTED: 9 (90%)
- INFERRED: 1 (10%)
- AMBIGUOUS: 0 (0%)

---

*Part of the graphify knowledge wiki. See [index](index.md) to navigate.*