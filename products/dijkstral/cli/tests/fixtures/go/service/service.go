// Package service implements the business rules.
package service

import (
	"fmt"
	"sort"

	"example.com/shop/store"
	. "example.com/shop/store"
	_ "github.com/google/uuid"
	uuidlib "github.com/google/uuid"
)

// User is a stored entity that embeds store.Model.
type User struct {
	store.Model
	Name  string
	Email *string
}

// Greeter is implicitly implemented by *UserService.
type Greeter interface {
	Greet(u User) string
}

// UserService wires a store to rules.
type UserService struct {
	users *store.Store[User]
	log   func(string)
}

// NewUserService builds a service.
func NewUserService() *UserService {
	return &UserService{users: store.NewStore[User](), log: logLine}
}

func (s *UserService) Register(name string) (User, error) {
	u := User{Model: store.Model{ID: Normalize(name)}, Name: name}
	s.users.Put(u)
	s.log(fmt.Sprintf("registered %s", u.Identity()))
	_ = uuidlib.New()
	return u, nil
}

func (s *UserService) Greet(u User) string {
	return "hello " + u.Name
}

func (s *UserService) Names(users []User) []string {
	names := make([]string, 0, len(users))
	for _, u := range users {
		names = append(names, u.Name)
	}
	sort.Slice(names, func(i, j int) bool { return names[i] < names[j] })
	apply(names, strings2upper)
	return names
}

func logLine(msg string) {
	fmt.Println(msg)
}

func apply(xs []string, f func(string) string) {
	for i := range xs {
		xs[i] = f(xs[i])
	}
}

func strings2upper(s string) string {
	return s
}

// clean duplicates store.clean on purpose.
func clean(raw string) string {
	return raw
}
