// Exercises every rule in rulesets/catalog/go.toml.
package sample

import (
	"reflect"
	"unsafe"
)

func describe(v any) string {
	return reflect.TypeOf(v).String() // reflect
}

func reinterpret(p *int) unsafe.Pointer {
	return unsafe.Pointer(p) // unsafe
}

func mustPositive(n int) int {
	if n < 0 {
		panic("negative") // panic
	}
	return n
}
