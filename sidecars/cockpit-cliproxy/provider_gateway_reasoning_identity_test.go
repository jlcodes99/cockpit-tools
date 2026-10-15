package main

import (
	"fmt"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/tidwall/gjson"
)

func TestProviderGatewayResponsesStreamPreservesReasoningIdentity(t *testing.T) {
	for _, id := range []string{"rs_original", "rs_" + strings.Repeat("a", 80), "third-party-id", " rs_original "} {
		t.Run(id, func(t *testing.T) {
			signature := validGPTReasoningTestSignature()
			// The signature is only available when the item completes. Its ID
			// must already have been preserved in the unsigned first snapshot.
			item := fmt.Sprintf(`{"type":"reasoning","id":%q,"summary":[],"encrypted_content":%q}`, id, signature)
			body := strings.Join([]string{
				fmt.Sprintf(`data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":%q,"summary":[]}}`, id),
				fmt.Sprintf(`data: {"type":"response.reasoning_summary_text.delta","item_id":%q,"delta":"thinking"}`, id),
				`data: {"type":"response.output_item.done","item":` + item + `}`,
				`data: {"type":"response.completed","response":{"output":[` + item + `]}}`,
			}, "\n\n") + "\n\n"
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			(&relayServer{}).writeProviderGatewayResponsesStream(c, strings.NewReader(body), false)
			if got := w.Body.String(); got != body {
				t.Fatalf("reasoning stream identity or ciphertext changed:\n%s", got)
			}
		})
	}
}

func TestProviderGatewayReasoningIdentitySurvivesReplay(t *testing.T) {
	for _, id := range []string{"rs_original", "third-party-id", strings.Repeat("a", 64)} {
		t.Run(id, func(t *testing.T) {
			item := fmt.Sprintf(`{"type":"reasoning","id":%q,"summary":[],"encrypted_content":%q}`, id, validGPTReasoningTestSignature())
			payload := []byte(`{"output":[` + item + `,{"type":"function_call","id":"tool-id","call_id":"call_1","name":"lookup","arguments":"{}"}]}`)
			out := newProviderGatewayItemIDRewriter().RewritePayload(normalizeResponsesReasoningContentBody(payload))
			if got := gjson.GetBytes(out, "output.0").Raw; got != item {
				t.Fatalf("reasoning item changed: %s", got)
			}
			if got := gjson.GetBytes(out, "output.1.id").String(); got != "fc_tool-id" {
				t.Fatalf("tool compatibility lost: %q", got)
			}
			input := []byte(`{"input":` + gjson.GetBytes(out, "output").Raw + `}`)
			if got := gjson.GetBytes(helps.SanitizeCodexInputItemIDs(input), "input.0").Raw; got != item {
				t.Fatalf("reasoning identity or ciphertext changed on replay: %s", got)
			}
		})
	}
}

func TestProviderGatewayReasoningDoesNotInventSignedID(t *testing.T) {
	for _, idField := range []string{"", `,"id":""`} {
		payload := []byte(`{"output":[{"type":"reasoning"` + idField + `,"encrypted_content":"opaque-ciphertext"}]}`)
		if got := newProviderGatewayItemIDRewriter().RewritePayload(payload); string(got) != string(payload) {
			t.Fatalf("invented identity for encrypted reasoning: %s", got)
		}
	}
}
