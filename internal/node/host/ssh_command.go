package host

import (
	"strings"
)

func shellQuote(value string) (string, error) {
	if strings.ContainsRune(value, '\x00') {
		return "", invalid("shell argument contains a null byte")
	}
	return "'" + strings.ReplaceAll(value, "'", "'\"'\"'") + "'", nil
}
