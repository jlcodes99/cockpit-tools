package cmd

import (
	"context"
	"io"
	"net/http"
	"reflect"
	"strings"
	"testing"
)

type projectListTransport func(*http.Request) (*http.Response, error)

func (f projectListTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func TestFetchGCPProjectsDecodesProjectList(t *testing.T) {
	client := &http.Client{Transport: projectListTransport(func(r *http.Request) (*http.Response, error) {
		if r.Method != http.MethodGet || r.URL.String() != "https://cloudresourcemanager.googleapis.com/v1/projects" {
			t.Fatalf("unexpected project request: %s %s", r.Method, r.URL)
		}
		return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(
			`{"projects":[{"projectId":"project-one","name":"First project","projectNumber":"123"},{"projectId":"project-two","name":"Second project"}]}`,
		))}, nil
	})}
	got, err := fetchGCPProjects(context.Background(), client)
	want := []gcpProject{{ProjectID: "project-one", Name: "First project"}, {ProjectID: "project-two", Name: "Second project"}}
	if err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("projects = %#v, error = %v", got, err)
	}
}

func TestFetchGCPProjectsRejectsFailedOrMalformedResponse(t *testing.T) {
	for _, tt := range []struct {
		name   string
		status int
		body   string
	}{
		{"http error", http.StatusForbidden, `{"error":"denied"}`},
		{"malformed JSON", http.StatusOK, `{"projects":`},
	} {
		t.Run(tt.name, func(t *testing.T) {
			client := &http.Client{Transport: projectListTransport(func(*http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: tt.status, Body: io.NopCloser(strings.NewReader(tt.body))}, nil
			})}
			if _, err := fetchGCPProjects(context.Background(), client); err == nil {
				t.Fatal("expected project discovery failure")
			}
		})
	}
}

func TestResolveGCPProjectSelectionsKeepsExplicitAndAllModes(t *testing.T) {
	projects := []gcpProject{{ProjectID: "one"}, {ProjectID: "two"}, {ProjectID: "one"}, {ProjectID: " "}}
	for _, tt := range []struct {
		selection string
		want      []string
		wantErr   bool
	}{
		{"ALL", []string{"one", "two"}, false},
		{"two,one,two", []string{"two", "one"}, false},
		{"unknown", nil, true},
	} {
		t.Run(tt.selection, func(t *testing.T) {
			got, err := resolveProjectSelections(tt.selection, projects)
			if (err != nil) != tt.wantErr || !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("selection = %#v, error = %v", got, err)
			}
		})
	}
}
