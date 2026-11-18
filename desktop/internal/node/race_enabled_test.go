//go:build race

package node

// raceInstrumented reports whether this test binary was built with the race
// detector. Instrumented builds are roughly an order of magnitude slower, so
// the multi-turn simulations that drive a whole goal loop skip themselves
// there; CI runs them uninstrumented and runs `-race` on the remaining tests.
const raceInstrumented = true
