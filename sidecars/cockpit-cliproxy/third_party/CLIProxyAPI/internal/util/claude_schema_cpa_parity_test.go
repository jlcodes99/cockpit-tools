package util

import "testing"

func TestCPAOctalNULRegexEscaping(t *testing.T) {
	for _, tc := range []struct {
		pattern     string
		unsupported bool
	}{
		{`^[^\0]*$`, true}, {`^\\0$`, false}, {`^[^\x00]*$`, false},
		{`\p{L}+`, true}, {`\\p{L}+`, false}, {`abc\`, false},
	} {
		if got := HasUnsupportedUnicodePropertyEscape(tc.pattern); got != tc.unsupported {
			t.Errorf("%q: unsupported=%v, want %v", tc.pattern, got, tc.unsupported)
		}
	}
}
