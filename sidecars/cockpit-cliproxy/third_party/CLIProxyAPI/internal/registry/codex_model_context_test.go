package registry

import (
	"reflect"
	"sync"
	"testing"
)

func preserveCodexContextCatalog(t *testing.T) {
	t.Helper()
	original, _ := GetCodexClientModelsSnapshot()
	t.Cleanup(func() {
		if _, err := loadCodexClientModelsFromBytes(original, "restore context test catalog"); err != nil {
			t.Fatalf("restore context catalog: %v", err)
		}
	})
}

func contextCatalogForTest(t *testing.T, window, maximum, compact int) []byte {
	t.Helper()
	sol := testCodexClientModel("gpt-5.6-sol", 1)
	luna := testCodexClientModel("gpt-5.6-luna", 2)
	for _, model := range []map[string]any{sol, luna} {
		model["context_window"] = window
		model["max_context_window"] = maximum
		model["auto_compact_token_limit"] = compact
	}
	return testCodexClientCatalog(t, sol, luna)
}

func TestCodexModelContextTemplateCachedLookupsAreIsolated(t *testing.T) {
	preserveCodexContextCatalog(t)
	data := contextCatalogForTest(t, 272000, 872000, 244800)
	if _, err := loadCodexClientModelsFromBytes(data, "context lookup test"); err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"gpt-5.6-luna", " GPT-5.6-LUNA ", " GPT-RESERVE "} {
		got, found := GetCodexModelContextTemplate(id)
		if !found || got.ContextWindow != 272000 || got.MaxContextWindow != 872000 ||
			got.AutoCompactTokenLimit == nil || *got.AutoCompactTokenLimit != 244800 {
			t.Fatalf("context template for %q = %#v, found %v", id, got, found)
		}
		*got.AutoCompactTokenLimit = 1
	}
	if got, found := GetCodexModelContextTemplate("unknown-model"); found || !reflect.DeepEqual(got, CodexModelContextTemplate{}) {
		t.Fatalf("unknown context template = %#v, found %v", got, found)
	}
	// A cached lookup may allocate its defensive compact-limit copy, but must
	// not allocate a catalog snapshot or decode its model entries again.
	if allocations := testing.AllocsPerRun(100, func() {
		GetCodexModelContextTemplate("gpt-5.6-sol")
	}); allocations > 1 {
		t.Fatalf("cached context lookup allocated %g times, want at most 1", allocations)
	}
	if allocations := testing.AllocsPerRun(100, func() {
		GetCodexModelContextTemplate("unknown-model")
	}); allocations != 0 {
		t.Fatalf("unknown context lookup allocated %g times, want 0", allocations)
	}
}

func TestCodexModelContextTemplatesFollowCatalogRevision(t *testing.T) {
	preserveCodexContextCatalog(t)
	first := contextCatalogForTest(t, 272000, 872000, 244800)
	if _, err := loadCodexClientModelsFromBytes(first, "first context revision"); err != nil {
		t.Fatal(err)
	}
	_, revision := GetCodexClientModelsSnapshot()
	if changed, err := loadCodexClientModelsFromBytes(first, "unchanged context revision"); err != nil || changed {
		t.Fatalf("unchanged catalog load = %v, %v", changed, err)
	}
	if got := GetCodexClientModelsRevision(); got != revision {
		t.Fatalf("unchanged catalog advanced revision from %d to %d", revision, got)
	}
	second := contextCatalogForTest(t, 516000, 1000000, 464400)
	if changed, err := loadCodexClientModelsFromBytes(second, "second context revision"); err != nil || !changed {
		t.Fatalf("updated catalog load = %v, %v", changed, err)
	}
	if got := GetCodexClientModelsRevision(); got != revision+1 {
		t.Fatalf("updated catalog revision = %d, want %d", got, revision+1)
	}
	updated, found := GetCodexModelContextTemplate("gpt-reserve")
	if !found || updated.ContextWindow != 516000 || updated.MaxContextWindow != 1000000 ||
		updated.AutoCompactTokenLimit == nil || *updated.AutoCompactTokenLimit != 464400 {
		t.Fatalf("updated Reserve context template = %#v, found %v", updated, found)
	}
	invalidCompact := testCodexClientModel("gpt-5.6-sol", 1)
	invalidCompact["auto_compact_token_limit"] = "invalid"
	for _, invalid := range [][]byte{[]byte(`{"models":[]}`), testCodexClientCatalog(t, invalidCompact)} {
		if _, err := loadCodexClientModelsFromBytes(invalid, "invalid context revision"); err == nil {
			t.Fatal("invalid catalog load succeeded")
		}
		if got, found := GetCodexModelContextTemplate("gpt-reserve"); !found || !reflect.DeepEqual(got, updated) {
			t.Fatalf("invalid catalog replaced context template: %#v, found %v", got, found)
		}
		if got := GetCodexClientModelsRevision(); got != revision+1 {
			t.Fatalf("invalid catalog advanced revision to %d", got)
		}
	}
	// Optional templates must disappear with the catalog, without leaving a
	// stale Reserve alias or inventing an automatic fallback.
	withoutLuna := testCodexClientCatalog(t, testCodexClientModel("gpt-5.6-sol", 1))
	if _, err := loadCodexClientModelsFromBytes(withoutLuna, "context revision without Luna"); err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"gpt-5.6-luna", "gpt-reserve"} {
		if got, found := GetCodexModelContextTemplate(id); found || !reflect.DeepEqual(got, CodexModelContextTemplate{}) {
			t.Fatalf("removed context template %q remains: %#v, found %v", id, got, found)
		}
	}
	if got, found := GetCodexModelContextTemplate("gpt-5.6-sol"); !found || got.AutoCompactTokenLimit != nil {
		t.Fatalf("optional compact limit was not preserved: %#v, found %v", got, found)
	}
}

func TestCodexModelContextTemplatesConcurrentRefresh(t *testing.T) {
	preserveCodexContextCatalog(t)
	first := contextCatalogForTest(t, 272000, 872000, 244800)
	second := contextCatalogForTest(t, 516000, 1000000, 464400)
	if _, err := loadCodexClientModelsFromBytes(first, "initial concurrent context revision"); err != nil {
		t.Fatal(err)
	}
	var workers sync.WaitGroup
	start := make(chan struct{})
	for worker := 0; worker < 8; worker++ {
		workers.Go(func() {
			<-start
			for range 1000 {
				got, found := GetCodexModelContextTemplate("gpt-reserve")
				if !found || got.AutoCompactTokenLimit == nil {
					t.Errorf("concurrent lookup lost its template: %#v, found %v", got, found)
					return
				}
				compact := *got.AutoCompactTokenLimit
				firstRevision := got.ContextWindow == 272000 && got.MaxContextWindow == 872000 && compact == 244800
				secondRevision := got.ContextWindow == 516000 && got.MaxContextWindow == 1000000 && compact == 464400
				if !firstRevision && !secondRevision {
					t.Errorf("concurrent lookup mixed catalog revisions: %#v, compact %d", got, compact)
					return
				}
				*got.AutoCompactTokenLimit = 1
			}
		})
	}
	workers.Go(func() {
		<-start
		for range 20 {
			for _, data := range [][]byte{second, first} {
				if _, err := loadCodexClientModelsFromBytes(data, "concurrent context revision"); err != nil {
					t.Errorf("concurrent catalog refresh: %v", err)
					return
				}
			}
		}
	})
	close(start)
	workers.Wait()
}

func BenchmarkCodexModelContextTemplate(b *testing.B) {
	for _, id := range []string{"gpt-5.6-sol", "gpt-reserve", "unknown-model"} {
		b.Run(id, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				GetCodexModelContextTemplate(id)
			}
		})
	}
}

func BenchmarkCodexModelContextConsumers(b *testing.B) {
	b.Run("known-model", func(b *testing.B) {
		b.ReportAllocs()
		for b.Loop() {
			LookupStaticModelInfo("gpt-5.6-sol")
		}
	})
	b.Run("unknown-model", func(b *testing.B) {
		b.ReportAllocs()
		for b.Loop() {
			LookupStaticModelInfo("deepseek-v4-pro")
		}
	})
	b.Run("paid-catalog", func(b *testing.B) {
		b.ReportAllocs()
		for b.Loop() {
			GetCodexProModels()
		}
	})
}
