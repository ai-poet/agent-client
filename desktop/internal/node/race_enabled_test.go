//go:build race

package node

// raceInstrumented reports whether this test binary was built with the race
// detector. testBudget uses it to give instrumented operations more wall-clock
// time without changing their uninstrumented budgets.
const raceInstrumented = true
