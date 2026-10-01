// Command shop exercises package pay.
package main

import (
	"fmt"

	"example.com/shop/internal/pay"
)

func main() {
	methods := []pay.Method{pay.Card{Fee: 2}, pay.Cash{}}
	for _, m := range methods {
		fmt.Println(pay.Settle(m, 12))
	}

	card := pay.Card{Fee: 1}
	charge := card.Charge // a method value
	fmt.Println(charge(10))

	double := func(x int) int { return 2 * x } // a closure
	fmt.Println(apply(double, 4))

	done := make(chan int)
	go func() { // a goroutine
		done <- pay.Sum([]int{1, 2, 3})
	}()
	fmt.Println(<-done)

	fmt.Println(pay.ChargeAll([]pay.Cash{{}}, 7))
}

// apply calls f: a call through a function value.
func apply(f func(int) int, x int) int {
	return f(x)
}
