package pay

// Number is the constraint of Sum.
type Number interface {
	~int | ~int64 | ~float64
}

// Sum adds xs.
func Sum[T Number](xs []T) T {
	var total T
	for _, x := range xs {
		total += x
	}
	return total
}

// ChargeAll charges amount through every method in ms: a call on a type
// parameter.
func ChargeAll[M Method](ms []M, amount int) []int {
	out := make([]int, 0, len(ms))
	for _, m := range ms {
		out = append(out, m.Charge(amount))
	}
	return out
}
