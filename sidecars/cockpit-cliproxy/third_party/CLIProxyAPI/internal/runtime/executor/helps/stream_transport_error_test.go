package helps

import (
	"strings"
	"testing"
)

func TestRedactStreamTransportErrorPreservesCauseWithoutCredentials(t *testing.T) {
	for _, message := range []string{
		"token='private secret'; unexpected EOF",
		"password:'foo bar'; unexpected EOF",
		`passwd="foo bar"; unexpected EOF`,
		"authorization: Basic Zm9vOmJhcg==; unexpected EOF",
		"Authorization=Bearer private-token; unexpected EOF",
		"GET https://user:pass@local.test/path?query=private%20secret; unexpected EOF",
		"GET wss://user:pass@local.test/?token=private; unexpected EOF",
		"read\r\nAuthorization: Basic Zm9vOmJhcg==\tunexpected EOF\x00",
	} {
		clean := RedactStreamTransportError(message)
		if !strings.Contains(clean, "unexpected EOF") || !strings.Contains(clean, "REDACTED") {
			t.Fatalf("cause lost: %q", clean)
		}
		for _, secret := range []string{"private", "foo bar", "Zm9vOmJhcg", "user:pass", "local.test", "\r", "\n", "\t", "\x00"} {
			if strings.Contains(clean, secret) {
				t.Fatalf("unsafe transport error: %q", clean)
			}
		}
	}
}
