package main

import (
	"encoding/json"
	"testing"
	"time"
)

func quotaCapacityWindow(value int, minutes int64) *quotaPoolWindowState {
	present := true
	return &quotaPoolWindowState{Present: &present, RemainingPercent: &value, WindowMinutes: &minutes}
}

func requireQuotaPercent(t *testing.T, label string, actual *int, expected int) {
	t.Helper()
	if actual == nil || *actual != expected {
		t.Fatalf("%s = %#v, want %d", label, actual, expected)
	}
}

func TestBuildCockpitQuotaCapacityUsesSameScopedWindowsAsRemaining(t *testing.T) {
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"a":       {Primary: quotaCapacityWindow(100, 300), Secondary: quotaCapacityWindow(90, 10080)},
		"b":       {Primary: quotaCapacityWindow(100, 300), Secondary: quotaCapacityWindow(80, 10080)},
		"c":       {Primary: quotaCapacityWindow(95, 300), Secondary: quotaCapacityWindow(70, 10080)},
		"outside": {Primary: quotaCapacityWindow(100, 300)},
		"api":     {Primary: quotaCapacityWindow(100, 300)},
	}}
	response := buildCockpitQuotaResponseWithAccounts(
		&apiKeySpec{AccountIDs: []string{"a", "b", "c", "a", "missing", "api"}}, state, time.Now(),
		map[string]*accountSpec{"api": {ID: "api", AuthKind: "api_key"}},
	)
	requireQuotaPercent(t, "hourly remaining", response.FiveHourRemainingPercent, 295)
	requireQuotaPercent(t, "hourly capacity", response.FiveHourCapacityPercent, 300)
	requireQuotaPercent(t, "weekly remaining", response.WeeklyRemainingPercent, 240)
	requireQuotaPercent(t, "weekly capacity", response.WeeklyCapacityPercent, 300)
	if response.AccountCount != 4 || response.MissingAccountCount != 1 {
		t.Fatalf("scope and duplicate semantics changed: %#v", response)
	}
}

func TestBuildCockpitQuotaCapacityExcludesMissingInvalidAndOtherWindows(t *testing.T) {
	missingPercent := quotaCapacityWindow(60, 300)
	missingPercent.RemainingPercent = nil
	absent := quotaCapacityWindow(80, 10080)
	present := false
	absent.Present = &present
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"hourly":  {Primary: quotaCapacityWindow(60, 300)},
		"weekly":  {Primary: quotaCapacityWindow(80, 10080)},
		"invalid": {Primary: missingPercent, Secondary: absent},
		"other":   {Primary: quotaCapacityWindow(50, 1440)},
	}}
	response := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: []string{"hourly", "weekly", "invalid", "other"}}, state, time.Now())
	requireQuotaPercent(t, "hourly capacity", response.FiveHourCapacityPercent, 100)
	requireQuotaPercent(t, "weekly capacity", response.WeeklyCapacityPercent, 100)
	for _, id := range []string{"missing", "invalid", "other"} {
		unknown := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: []string{id}}, state, time.Now())
		if unknown.FiveHourCapacityPercent != nil || unknown.WeeklyCapacityPercent != nil {
			t.Fatalf("unknown windows acquired a capacity: id=%s response=%#v", id, unknown)
		}
		encoded, err := json.Marshal(unknown)
		if err != nil {
			t.Fatal(err)
		}
		var fields map[string]json.RawMessage
		if err = json.Unmarshal(encoded, &fields); err != nil {
			t.Fatal(err)
		}
		if fields["fiveHourCapacityPercent"] != nil || fields["weeklyCapacityPercent"] != nil {
			t.Fatal("unknown capacities must remain absent in the endpoint JSON")
		}
	}
}

func TestBuildCockpitQuotaCapacityDistinguishesExhaustedFromEmpty(t *testing.T) {
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"exhausted": {Primary: quotaCapacityWindow(100, 300), Secondary: quotaCapacityWindow(0, 10080)},
	}}
	response := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: []string{"exhausted"}}, state, time.Now())
	requireQuotaPercent(t, "hourly remaining", response.FiveHourRemainingPercent, 0)
	requireQuotaPercent(t, "weekly remaining", response.WeeklyRemainingPercent, 0)
	requireQuotaPercent(t, "hourly capacity", response.FiveHourCapacityPercent, 100)
	requireQuotaPercent(t, "weekly capacity", response.WeeklyCapacityPercent, 100)
	empty := buildCockpitQuotaResponse(&apiKeySpec{}, state, time.Now())
	requireQuotaPercent(t, "empty hourly capacity", empty.FiveHourCapacityPercent, 0)
	requireQuotaPercent(t, "empty weekly capacity", empty.WeeklyCapacityPercent, 0)
	encoded, err := json.Marshal(empty)
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]json.RawMessage
	if err = json.Unmarshal(encoded, &fields); err != nil {
		t.Fatal(err)
	}
	if string(fields["fiveHourCapacityPercent"]) != "0" || string(fields["weeklyCapacityPercent"]) != "0" {
		t.Fatal("known empty capacities must be serialized as zero")
	}
}

func TestBuildCockpitQuotaCapacityTracksTwoShortWindowsFromOneAccount(t *testing.T) {
	state := quotaPoolStateFile{Accounts: map[string]quotaPoolAccountState{
		"two": {Primary: quotaCapacityWindow(60, 300), Secondary: quotaCapacityWindow(80, 120)},
	}}
	response := buildCockpitQuotaResponse(&apiKeySpec{AccountIDs: []string{"two"}}, state, time.Now())
	requireQuotaPercent(t, "hourly remaining", response.FiveHourRemainingPercent, 140)
	requireQuotaPercent(t, "hourly capacity", response.FiveHourCapacityPercent, 200)
	if response.AccountCount != 1 || response.WeeklyCapacityPercent != nil {
		t.Fatalf("capacity must follow windows rather than account count: %#v", response)
	}
}
