//go:build !race

package node

// raceInstrumented reports whether this test binary was built with the race
// detector. See race_enabled_test.go.
const raceInstrumented = false
