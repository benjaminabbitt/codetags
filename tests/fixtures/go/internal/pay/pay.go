// Package pay holds payment methods: an interface with two implementations.
package pay

// Method charges an amount and returns what it took.
type Method interface {
	Charge(amount int) int
}

// Card adds a fixed fee.
type Card struct {
	Fee int
}

// Charge implements Method.
func (c Card) Charge(amount int) int {
	return amount + c.Fee
}

// Cash rounds down to a multiple of five.
type Cash struct{}

// Charge implements Method.
func (Cash) Charge(amount int) int {
	return amount - amount%5
}

// Settle charges amount through m: the interface call site.
func Settle(m Method, amount int) int {
	return m.Charge(amount)
}
