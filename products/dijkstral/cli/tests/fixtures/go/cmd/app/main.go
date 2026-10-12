package main

import (
	"fmt"
	"os"

	"example.com/shop/service"
	"example.com/shop/store"
)

func main() {
	svc := service.NewUserService()
	u, err := svc.Register(os.Args[0])
	if err != nil {
		os.Exit(1)
	}
	var g service.Greeter = svc
	fmt.Println(g.Greet(u), store.Sum([]int{1, 2}), store.Format(3))
	run := func() { fmt.Println("closure") }
	run()
}
