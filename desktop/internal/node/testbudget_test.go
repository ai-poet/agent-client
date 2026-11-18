package node

import "time"

const baseTestBudget = 30 * time.Second

// testBudget is the wall-clock budget a fixture gives an operation. Race
// builds run roughly 4x slower, so the budget scales there instead of every
// heavy test skipping itself.
func testBudget() time.Duration {
	if raceInstrumented {
		return baseTestBudget * 4
	}
	return baseTestBudget
}
