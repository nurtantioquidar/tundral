package service

import "testing"

func TestRegister(t *testing.T) {
	svc := NewUserService()
	u, err := svc.Register(clean(" Alice "))
	if err != nil || u.Name == "" {
		t.Fatal("register failed")
	}
}
