package executor

import (
	"net/http"
	"testing"

	"github.com/tidwall/gjson"
)

// --- max_tokens clamping ---------------------------------------------------

func TestClampCodebuddyMaxTokens_ClampsOversizedValue(t *testing.T) {
	out := clampCodebuddyMaxTokens([]byte(`{"max_tokens":65536}`), "deepseek-v4-pro")
	got := gjson.GetBytes(out, "max_tokens").Int()
	if got != 32768 {
		t.Fatalf("max_tokens = %d, want 32768; out=%s", got, out)
	}
}

func TestClampCodebuddyMaxTokens_ClampsMaxCompletionTokensAlias(t *testing.T) {
	out := clampCodebuddyMaxTokens([]byte(`{"max_completion_tokens":65536}`), "deepseek-v4-pro")
	got := gjson.GetBytes(out, "max_completion_tokens").Int()
	if got != 32768 {
		t.Fatalf("max_completion_tokens = %d, want 32768; out=%s", got, out)
	}
}

func TestClampCodebuddyMaxTokens_LeavesValueAtOrBelowCeiling(t *testing.T) {
	out := clampCodebuddyMaxTokens([]byte(`{"max_tokens":100}`), "deepseek-v4-pro")
	if got := gjson.GetBytes(out, "max_tokens").Int(); got != 100 {
		t.Fatalf("max_tokens = %d, want 100; out=%s", got, out)
	}
}

func TestClampCodebuddyMaxTokens_NoFieldUnchanged(t *testing.T) {
	in := `{"model":"deepseek-v4-pro"}`
	out := clampCodebuddyMaxTokens([]byte(in), "deepseek-v4-pro")
	if string(out) != in {
		t.Fatalf("expected unchanged, got %s", out)
	}
}

func TestClampCodebuddyMaxTokens_StringValueNotClamped(t *testing.T) {
	in := `{"max_tokens":"65536"}`
	out := clampCodebuddyMaxTokens([]byte(in), "deepseek-v4-pro")
	if got := gjson.GetBytes(out, "max_tokens").String(); got != "65536" {
		t.Fatalf("string max_tokens should be untouched, got %s", got)
	}
}

// --- quota business-code -> HTTP 429 mapping -------------------------------

func TestCodebuddyEffectiveStatus_QuotaTopLevelCodeMapsTo429(t *testing.T) {
	body := []byte(`{"code":14018,"msg":"额度已用尽"}`)
	if got := codebuddyEffectiveStatus(http.StatusBadRequest, body); got != http.StatusTooManyRequests {
		t.Fatalf("status = %d, want %d", got, http.StatusTooManyRequests)
	}
}

func TestCodebuddyEffectiveStatus_QuotaNestedCodeMapsTo429(t *testing.T) {
	body := []byte(`{"error":{"data":{"code":14018,"msg":"额度已用尽"}}}`)
	if got := codebuddyEffectiveStatus(http.StatusBadRequest, body); got != http.StatusTooManyRequests {
		t.Fatalf("status = %d, want %d", got, http.StatusTooManyRequests)
	}
}

func TestCodebuddyEffectiveStatus_InvalidParamKeeps400(t *testing.T) {
	body := []byte(`{"code":11133,"msg":"Invalid request parameters"}`)
	if got := codebuddyEffectiveStatus(http.StatusBadRequest, body); got != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", got, http.StatusBadRequest)
	}
}

func TestCodebuddyEffectiveStatus_ServerErrorUnchanged(t *testing.T) {
	if got := codebuddyEffectiveStatus(http.StatusInternalServerError, []byte(`{}`)); got != http.StatusInternalServerError {
		t.Fatalf("status = %d, want %d", got, http.StatusInternalServerError)
	}
}

func TestCodebuddyEffectiveStatus_SuccessUntouched(t *testing.T) {
	body := []byte(`{"code":14018}`)
	if got := codebuddyEffectiveStatus(http.StatusOK, body); got != http.StatusOK {
		t.Fatalf("status = %d, want %d", got, http.StatusOK)
	}
}

// --- agentic tool_choice reset ---------------------------------------------

func TestInjectCodebuddyInspectTool_ResetsStaleToolChoice(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","tool_choice":{"type":"function","function":{"name":"read_file"}},"messages":[{"role":"user","content":"hi"}]}`)
	out := injectCodebuddyInspectTool(body, 1)
	if got := gjson.GetBytes(out, "tool_choice").String(); got != "auto" {
		t.Fatalf("tool_choice = %q, want %q; out=%s", got, "auto", out)
	}
}

func TestInjectCodebuddyInspectTool_ResetsRequiredToolChoice(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","tool_choice":"required","messages":[{"role":"user","content":"hi"}]}`)
	out := injectCodebuddyInspectTool(body, 1)
	if got := gjson.GetBytes(out, "tool_choice").String(); got != "auto" {
		t.Fatalf("tool_choice = %q, want %q; out=%s", got, "auto", out)
	}
}

func TestInjectCodebuddyInspectTool_AddsToolChoiceWhenAbsent(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","messages":[{"role":"user","content":"hi"}]}`)
	out := injectCodebuddyInspectTool(body, 1)
	if got := gjson.GetBytes(out, "tool_choice").String(); got != "auto" {
		t.Fatalf("tool_choice = %q, want %q; out=%s", got, "auto", out)
	}
}
