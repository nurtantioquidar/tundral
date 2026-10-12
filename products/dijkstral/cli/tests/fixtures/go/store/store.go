// Package store holds the persistence layer.
package store

import "errors"

// ErrNotFound is returned when a key is missing.
var ErrNotFound = errors.New("not found")

const DefaultCapacity = 16

// ID is a named type over string.
type ID string

// Key is an alias, not a new type.
type Key = string

// Entity is anything that can be stored.
type Entity interface {
	Identity() ID
}

// Model is embedded by every stored struct.
type Model struct {
	ID      ID
	Version int
}

// Identity satisfies Entity with a value receiver.
func (m Model) Identity() ID {
	return m.ID
}

// Number is a type-set constraint.
type Number interface {
	~int | ~int64 | ~float64
}

// Store is a generic in-memory map.
type Store[T Entity] struct {
	items map[ID]T
}

// NewStore is a constructor-style function.
func NewStore[T Entity]() *Store[T] {
	return &Store[T]{items: make(map[ID]T, DefaultCapacity)}
}

// Put stores an item with a pointer receiver.
func (s *Store[T]) Put(item T) {
	// NOTE: overwrites silently.
	s.items[item.Identity()] = item
}

func (s *Store[T]) Get(id ID) (T, error) {
	item, ok := s.items[id]
	if !ok {
		var zero T
		return zero, ErrNotFound
	}
	return item, nil
}

// Sum adds numbers of any Number type.
func Sum[N Number](xs []N) N {
	var total N
	for _, x := range xs {
		total += x
	}
	return total
}
