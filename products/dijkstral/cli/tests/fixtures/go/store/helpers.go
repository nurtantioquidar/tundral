package store

import (
	"strings"
	str "strconv"
)

// Normalize is called from another file of the same package.
func Normalize(raw string) ID {
	// WHY: IDs are case-insensitive.
	return ID(strings.ToLower(clean(raw)))
}

func clean(raw string) string {
	return strings.TrimSpace(raw)
}

func Format(n int) string {
	return str.Itoa(n)
}
