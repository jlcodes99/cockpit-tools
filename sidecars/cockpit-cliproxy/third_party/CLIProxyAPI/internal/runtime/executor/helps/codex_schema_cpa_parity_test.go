package helps

import (
	"encoding/json"
	"github.com/tidwall/gjson"
	"testing"
)

func TestNormalizeCodexToolSchemas_StripsOctalNULPatternEscape(t *testing.T) {
	// Claude Code's built-in Artifact tool guards file paths with the octal NUL
	// escape, e.g. "^[^\\0]*$" on the upload_asset file_paths item. Strict upstream
	// validators reject that spelling ("is not a 'regex'") while accepting the
	// equivalent \\x00, so the pattern has to come off before the request goes out.
	input := []byte(`{
		"model": "gpt-5.6",
		"tools": [{
			"type": "function",
			"name": "Artifact",
			"parameters": {
				"type": "object",
				"properties": {
					"file_paths": {
						"type": "array",
						"minItems": 1,
						"items": {
							"type": "string",
							"minLength": 1,
							"maxLength": 1024,
							"pattern": "^[^\\0]*$"
						}
					},
					"asset_id": {
						"type": "string",
						"pattern": "^[0-9a-f]{32}$"
					},
					"hex_nul": {
						"type": "string",
						"pattern": "^[^\\x00]*$"
					}
				},
				"required": ["file_paths"]
			}
		}]
	}`)

	out := NormalizeCodexToolSchemas(input)

	params := gjson.GetBytes(out, "tools.0.parameters")

	// The octal NUL pattern must be removed, while the rest of the item schema stays.
	if params.Get("properties.file_paths.items.pattern").Exists() {
		t.Errorf("expected properties.file_paths.items.pattern to be removed, got: %s",
			params.Get("properties.file_paths.items.pattern").Raw)
	}
	if got := params.Get("properties.file_paths.items.type").String(); got != "string" {
		t.Errorf("expected properties.file_paths.items.type == 'string', got %q", got)
	}
	for path, want := range map[string]string{
		"properties.file_paths.items.minLength": "1",
		"properties.file_paths.items.maxLength": "1024",
		"properties.file_paths.minItems":        "1",
	} {
		if got := params.Get(path).String(); got != want {
			t.Errorf("expected %s == %q, got %q", path, want, got)
		}
	}

	// A plain pattern is valid and must survive.
	if got := params.Get("properties.asset_id.pattern").String(); got != "^[0-9a-f]{32}$" {
		t.Errorf("expected properties.asset_id.pattern preserved, got %q", got)
	}

	// The hex NUL spelling is the one strict validators accept, so it must survive.
	if got := params.Get("properties.hex_nul.pattern").String(); got != `^[^\x00]*$` {
		t.Errorf("expected properties.hex_nul.pattern preserved, got %q", got)
	}

	// Idempotence test
	outAgain := NormalizeCodexToolSchemas(out)
	if string(outAgain) != string(out) {
		t.Errorf("expected NormalizeCodexToolSchemas to be idempotent")
	}
}

func TestNormalizeCodexToolSchemas_DoesNotNormalizeIntegerTypes(t *testing.T) {
	input := []byte(`{
		"tools": [
			{
				"type": "function",
				"name": "exec_command",
				"parameters": {
					"type": "object",
					"properties": {
						"yield_time_ms": {"type": "number"}
					}
				}
			}
		]
	}`)
	out := NormalizeCodexToolSchemas(input)
	if got := gjson.GetBytes(out, "tools.0.parameters.properties.yield_time_ms.type").String(); got != "number" {
		t.Fatalf("expected number, got: %s", got)
	}
}

func TestNormalizeCodexToolSchemas_StripsOutOfRangeInt64Bounds(t *testing.T) {
	input := []byte(`{
		"model": "gpt-6-luna",
		"tools": [{
			"type": "function",
			"name": "emulate_process",
			"description": "Emulate a process.",
			"parameters": {
				"type": "object",
				"properties": {
					"options": {
						"type": "object",
						"properties": {
							"seed": {
								"type": "integer",
								"minimum": 0,
								"maximum": 18446744073709552000
							},
							"over_max_int64": {
								"type": "integer",
								"minimum": -9223372036854775809,
								"maximum": 9223372036854775808,
								"exclusiveMinimum": -9223372036854775809,
								"exclusiveMaximum": 9223372036854775808
							},
							"exact_int64_limits": {
								"type": "integer",
								"minimum": -9223372036854775808,
								"maximum": 9223372036854775807,
								"exclusiveMinimum": -9223372036854775808,
								"exclusiveMaximum": 9223372036854775807
							},
							"float_bounds": {
								"type": "number",
								"minimum": -0.5,
								"maximum": 3.14,
								"exclusiveMinimum": 0.25,
								"exclusiveMaximum": 99.95
							},
							"scientific_bounds": {
								"type": "number",
								"minimum": 1e5,
								"maximum": 1e100,
								"exclusiveMinimum": -1.8446744073709552e+19,
								"exclusiveMaximum": 18446744073709552000.0
							},
							"high_precision_fraction": {
								"type": "number",
								"maximum": 18446744073709552000.00000000000000000000000000000000000000000000000000000000000000000000000000000001
							},
							"extreme_exponents": {
								"type": "number",
								"minimum": 1.1e-9223372036854775808,
								"maximum": 10e9223372036854775807
							},
							"user_data": {
								"type": "object",
								"default": {
									"maximum": 18446744073709552000
								},
								"enum": [
									{"maximum": 18446744073709552000}
								]
							},
							"maximum": {
								"type": "integer",
								"minimum": 0
							}
						}
					}
				},
				"$defs": {
					"uint64_id": {
						"type": "integer",
						"maximum": 18446744073709551615
					}
				}
			}
		}]
	}`)

	out := NormalizeCodexToolSchemas(input)
	params := gjson.GetBytes(out, "tools.0.parameters")
	optsProps := params.Get("properties.options.properties")

	// Issue #6579 reproduction: u64::MAX bound dropped, minimum: 0 and type preserved.
	if optsProps.Get("seed.maximum").Exists() {
		t.Errorf("expected options.seed.maximum to be removed, got %s", optsProps.Get("seed.maximum").Raw)
	}
	if got := optsProps.Get("seed.minimum").Raw; got != "0" {
		t.Errorf("expected options.seed.minimum == 0, got %s", got)
	}
	if got := optsProps.Get("seed.type").String(); got != "integer" {
		t.Errorf("expected options.seed.type == 'integer', got %q", got)
	}

	// All four bound keywords outside int64 must be dropped.
	for _, kw := range []string{"minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"} {
		if optsProps.Get("over_max_int64." + kw).Exists() {
			t.Errorf("expected over_max_int64.%s to be removed, got %s", kw, optsProps.Get("over_max_int64."+kw).Raw)
		}
	}

	// Exact int64 min/max must be preserved without precision loss.
	if got := optsProps.Get("exact_int64_limits.minimum").Raw; got != "-9223372036854775808" {
		t.Errorf("expected exact_int64_limits.minimum == -9223372036854775808, got %s", got)
	}
	if got := optsProps.Get("exact_int64_limits.maximum").Raw; got != "9223372036854775807" {
		t.Errorf("expected exact_int64_limits.maximum == 9223372036854775807, got %s", got)
	}
	if got := optsProps.Get("exact_int64_limits.exclusiveMinimum").Raw; got != "-9223372036854775808" {
		t.Errorf("expected exact_int64_limits.exclusiveMinimum == -9223372036854775808, got %s", got)
	}
	if got := optsProps.Get("exact_int64_limits.exclusiveMaximum").Raw; got != "9223372036854775807" {
		t.Errorf("expected exact_int64_limits.exclusiveMaximum == 9223372036854775807, got %s", got)
	}

	// Fractional float bounds must be preserved.
	if got := optsProps.Get("float_bounds.minimum").Raw; got != "-0.5" {
		t.Errorf("expected float_bounds.minimum == -0.5, got %s", got)
	}
	if got := optsProps.Get("float_bounds.maximum").Raw; got != "3.14" {
		t.Errorf("expected float_bounds.maximum == 3.14, got %s", got)
	}

	// Scientific/decimal integer bounds: in-range preserved, out-of-range dropped.
	if got := optsProps.Get("scientific_bounds.minimum").Raw; got != "1e5" {
		t.Errorf("expected scientific_bounds.minimum == 1e5, got %s", got)
	}
	for _, kw := range []string{"maximum", "exclusiveMinimum", "exclusiveMaximum"} {
		if optsProps.Get("scientific_bounds." + kw).Exists() {
			t.Errorf("expected scientific_bounds.%s to be removed, got %s", kw, optsProps.Get("scientific_bounds."+kw).Raw)
		}
	}

	// High-precision non-integer bound must not be rounded to an integer and dropped.
	wantHighPrec := "18446744073709552000.00000000000000000000000000000000000000000000000000000000000000000000000000000001"
	if got := optsProps.Get("high_precision_fraction.maximum").Raw; got != wantHighPrec {
		t.Errorf("expected high_precision_fraction.maximum preserved as %s, got %s", wantHighPrec, got)
	}

	// Extreme int64 exponents must not overflow effectiveExp calculation.
	if got := optsProps.Get("extreme_exponents.minimum").Raw; got != "1.1e-9223372036854775808" {
		t.Errorf("expected extreme_exponents.minimum preserved as 1.1e-9223372036854775808, got %s", got)
	}
	if optsProps.Get("extreme_exponents.maximum").Exists() {
		t.Errorf("expected extreme_exponents.maximum (10e9223372036854775807) to be removed, got %s", optsProps.Get("extreme_exponents.maximum").Raw)
	}

	// Non-schema user data under default/enum and property named "maximum" must be preserved.
	if got := optsProps.Get("user_data.default.maximum").Raw; got != "18446744073709552000" {
		t.Errorf("expected user_data.default.maximum preserved, got %s", got)
	}
	if got := optsProps.Get("user_data.enum.0.maximum").Raw; got != "18446744073709552000" {
		t.Errorf("expected user_data.enum.0.maximum preserved, got %s", got)
	}
	if got := optsProps.Get("maximum.type").String(); got != "integer" {
		t.Errorf("expected property named 'maximum' preserved, got %q", got)
	}

	// Subschemas under $defs must also have out-of-range integer bounds removed.
	if params.Get("$defs.uint64_id.maximum").Exists() {
		t.Errorf("expected $defs.uint64_id.maximum to be removed, got %s", params.Get("$defs.uint64_id.maximum").Raw)
	}

	// Idempotence check.
	outAgain := NormalizeCodexToolSchemas(out)
	if string(outAgain) != string(out) {
		t.Errorf("expected NormalizeCodexToolSchemas to be idempotent")
	}
}

func TestCPANumericBoundExactExponentAndFractionLimits(t *testing.T) {
	for _, tc := range []struct {
		raw    string
		remove bool
	}{
		{"9223372036854775807", false}, {"-9223372036854775808", false},
		{"9223372036854775808", true}, {"-9223372036854775809", true},
		{"9.223372036854775807e18", false}, {"9.223372036854775808e18", true},
		{"-9.223372036854775808e18", false}, {"-9.223372036854775809e18", true},
		{"9223372036854775808.5", false}, {"9223372036854775808.0", true},
		{"8e-1", false}, {"0e999999999999999999999999999999", false},
		{"1e999999999999999999999999999999", true}, {"1e-999999999999999999999999999999", false},
	} {
		if got := isNumericBoundOutOfInt64(json.Number(tc.raw)); got != tc.remove {
			t.Errorf("%s: remove=%v, want %v", tc.raw, got, tc.remove)
		}
	}
}
