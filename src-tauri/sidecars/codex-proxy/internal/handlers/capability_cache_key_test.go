package handlers

import "testing"

func TestBuildCapabilityCacheKeyIncludesModelsDimension(t *testing.T) {
	keyA := buildCapabilityCacheKey(
		"https://example.com",
		"sk-test",
		"responses",
		[]string{"responses", "messages"},
		[]string{"gpt-4o", "claude-3-7-sonnet"},
		"",
	)
	keyB := buildCapabilityCacheKey(
		"https://example.com",
		"sk-test",
		"responses",
		[]string{"messages", "responses"},
		[]string{"claude-3-7-sonnet", "gpt-4o"},
		"",
	)
	keyC := buildCapabilityCacheKey(
		"https://example.com",
		"sk-test",
		"responses",
		[]string{"messages", "responses"},
		[]string{"gpt-4o-mini"},
		"",
	)

	if keyA != keyB {
		t.Fatalf("same model set should yield same key, got %q != %q", keyA, keyB)
	}
	if keyA == keyC {
		t.Fatalf("different model set should yield different key, got %q", keyA)
	}
}

func TestBuildCapabilityCacheKeyIncludesModelMappingHash(t *testing.T) {
	base := buildCapabilityCacheKey(
		"https://example.com",
		"sk-test",
		"messages",
		[]string{"messages"},
		nil,
		"",
	)
	withHash := buildCapabilityCacheKey(
		"https://example.com",
		"sk-test",
		"messages",
		[]string{"messages"},
		nil,
		hashModelMapping(map[string]string{"claude-opus-4-7": "anthropic/claude-opus-4"}),
	)
	withDifferentHash := buildCapabilityCacheKey(
		"https://example.com",
		"sk-test",
		"messages",
		[]string{"messages"},
		nil,
		hashModelMapping(map[string]string{"claude-opus-4-7": "anthropic/claude-opus-4-v2"}),
	)

	if base == withHash {
		t.Fatalf("empty vs non-empty mapping should yield different keys, got %q", base)
	}
	if withHash == withDifferentHash {
		t.Fatalf("different mappings should yield different keys, got %q", withHash)
	}
}

func TestHashModelMappingStable(t *testing.T) {
	a := hashModelMapping(map[string]string{"a": "x", "b": "y"})
	b := hashModelMapping(map[string]string{"b": "y", "a": "x"})
	if a != b {
		t.Fatalf("hash should be order-independent, got %q vs %q", a, b)
	}
	if hashModelMapping(nil) != "" {
		t.Fatalf("nil mapping should produce empty hash")
	}
	if hashModelMapping(map[string]string{}) != "" {
		t.Fatalf("empty mapping should produce empty hash")
	}
}
