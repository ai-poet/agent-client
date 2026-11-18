package node

import (
	"fmt"
	"time"
)

const (
	baseTestBudget = 30 * time.Second
	// maxTestBudget caps the scaled budget: a genuinely hung test must not
	// take four times as long to report simply because it runs under -race.
	maxTestBudget = 3 * time.Minute
)

// testBudget is the wall-clock budget a fixture gives an operation. Race
// builds run roughly 4x slower, so the budget scales there instead of every
// heavy test skipping itself, but never past maxTestBudget.
func testBudget() time.Duration {
	if !raceInstrumented {
		return baseTestBudget
	}
	scaled := baseTestBudget * 4
	if scaled > maxTestBudget {
		return maxTestBudget
	}
	return scaled
}

// awaitBudget waits for cond until the test budget runs out, reporting both
// what it waited for and the budget it was given, so a slow run is not
// mistaken for a hang at a glance.
func awaitBudget(what string, cond func() bool) error {
	budget := testBudget()
	start := time.Now()
	deadline := start.Add(budget)
	for {
		if cond() {
			return nil
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("%s did not settle within %s (waited %s)", what, budget, time.Since(start).Round(time.Millisecond))
		}
		time.Sleep(5 * time.Millisecond)
	}
}
