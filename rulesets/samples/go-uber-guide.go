// Sample for rulesets/go-uber-guide.toml — every rule must fire at least
// once here; the good/compliant form sits next to the bad one wherever that
// is cheap. The package name itself is deliberately non-conforming so the
// package-name rule has something to fire on (see below).
package sample_pkg

import (
	"context"
	"errors"
	"fmt"
	"sync"
)

import _ "net/http/pprof" // bad: import blank
import . "fmt"            // bad: import dot

// ---- Avoid init() (uber: avoid-init) ----------------------------------

func init() { // bad: defines init
	// bad: goroutine started from init() (uber: no-goroutines-in-init)
	go doSetup()
}

func doSetup() {}

// ---- Don't Panic (uber: dont-panic) ------------------------------------

func riskyOp() {
	panic("boom") // bad: uses panic
}

// ---- Avoid Mutable Globals (uber: avoid-mutable-globals) ---------------

var requestCount = 0 // bad: mutable package-level var

const maxRetries = 3 // good: const, not flagged

// ---- Zero-value Mutexes are Valid (uber: zero-value-mutexes) -----------

type BadBox struct {
	mu *sync.Mutex // bad: pointer to sync.Mutex
}

type GoodBox struct {
	mu sync.Mutex // good: zero-value mutex
}

// ---- Avoid Embedding Types in Public Structs (uber: avoid-embedding) ---

type BadClient struct {
	Config // bad: embedded field, no name
}

type GoodClient struct {
	config Config // good: named field instead of embedding
}

type Config struct{}

// ---- Start Enums at One (uber: start-enums-at-one) ----------------------

type Operation int

const (
	Add      Operation = iota // bad: enum starts at zero
	Subtract
)

type Weekday int

const (
	Sunday Weekday = iota + 1 // good: starts at one
	Monday
)

// ---- Error Naming (uber: error-naming) -----------------------------------

var NotFound = errors.New("not found") // bad: sentinel not prefixed Err

var ErrNotFound = errors.New("not found") // good

// ---- Error Strings (crc: error-strings) ----------------------------------

var errBad = errors.New("Something went Wrong.") // bad: capitalized + punctuation

var errGood = errors.New("something went wrong") // good

// ---- Handle Type Assertion Failures (crc: handle-type-assertion) --------

func assertBad(i interface{}) {
	s := i.(string) // bad: unchecked type assertion
	_ = s
}

func assertGood(i interface{}) {
	s, ok := i.(string) // good: checked
	_ = s
	_ = ok
}

// ---- Channel Size is One or None (uber: channel-size) --------------------

func channels() {
	bad := make(chan int, 10) // bad: buffered beyond one
	_ = bad
	good := make(chan int, 1) // good
	_ = good
}

// ---- Declaring Empty Slices (crc: declaring-empty-slices) ---------------

func slices() {
	bad := []string{} // bad
	_ = bad
	var good []string // good
	_ = good
}

// ---- Unnecessary Else (uber: unnecessary-else) ---------------------------

func absBad(x int) int {
	if x > 0 {
		return x
	} else { // bad: else after a return
		return -x
	}
}

func absGood(x int) int {
	if x > 0 {
		return x
	}
	return -x // good: no else
}

// ---- Naked Returns (uber: naked-returns) ---------------------------------

func divideBad(a, b int) (result int) {
	result = a / b
	return // bad: naked return
}

func divideGood(a, b int) int {
	return a / b // good
}

// ---- Contexts first parameter (crc: contexts) -----------------------------

func doThingBad(x int, ctx context.Context) { // bad: ctx not first
	_ = x
	_ = ctx
}

func doThingGood(ctx context.Context, x int) { // good
	_ = x
	_ = ctx
}

// ---- Don't fire-and-forget goroutines (uber: fire-and-forget) -----------

func background() {
	go doSetup() // bad: no wait / error handling attached
}

// ---- Prefer strconv over fmt (uber: prefer-strconv) ----------------------

func toStringBad(x int) string {
	return fmt.Sprint(x) // bad
}

// ---- Avoid Using Built-In Names (uber: avoid-builtin-names) -------------

func countBad(len int) int { // bad: shadows builtin `len`
	return len
}

func countGood(count int) int { // good
	return count
}

// ---- Receiver Names (crc: receiver-names) --------------------------------

type Widget struct{}

func (this *Widget) Render() {} // bad: receiver named `this`

func (w *Widget) Draw() {} // good

// ---- Function Names / Mixed Caps (uber: function-names, crc: mixed-caps) -

func do_work() {} // bad: underscore in name

func DoWork() {} // good

// ---- Initialisms (crc: initialisms) ---------------------------------------

var UserId string // bad: should be UserID

var UserID string // good

// ---- Avoid Naked Parameters (uber: avoid-naked-parameters) ---------------

func newClient(retry bool, stream bool) {}

func callerBad() {
	newClient(true, false) // bad: naked bool literals
}
