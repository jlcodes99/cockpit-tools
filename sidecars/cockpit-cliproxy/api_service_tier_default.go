package main

import (
	"encoding/json"
	"io"
	"net"
	"net/http"
	"sync"
	"sync/atomic"

	"github.com/gin-gonic/gin"
)

const apiServiceTierDefaultPath = "/v1/cockpit/service-tier-default"

// This state belongs to one public API-service process. Provider gateways do
// not enable it, and active requests retain their own immutable snapshot.
type apiServiceTierDefault struct {
	value    atomic.Value // string: empty (Standard), priority (Fast), or ultrafast
	mu       sync.Mutex
	revision uint64
}

func newAPIServiceTierDefault(tier string) *apiServiceTierDefault {
	state := &apiServiceTierDefault{}
	normalized := normalizedUsageServiceTier(tier)
	if normalized != "priority" && normalized != "ultrafast" {
		normalized = ""
	}
	state.value.Store(normalized)
	return state
}

func (s *apiServiceTierDefault) tier() string {
	if tier, ok := s.value.Load().(string); ok {
		return tier
	}
	return ""
}

func (s *relayServer) handleAPIServiceTierDefault(c *gin.Context) {
	if !s.requireDiagnosticsControlKey(c) {
		return
	}
	host, _, err := net.SplitHostPort(c.Request.RemoteAddr)
	if err != nil || !net.ParseIP(host).IsLoopback() {
		writeAPIError(c, http.StatusForbidden, "local control connection required", "permission_denied")
		return
	}
	if !s.manifest.ServiceTierControlEnabled || s.policy == nil || s.policy.serviceTierDefault == nil {
		writeAPIError(c, http.StatusNotFound, "API service speed control is unavailable", "not_found")
		return
	}
	var input struct {
		Speed    string `json:"speed"`
		Revision uint64 `json:"revision"`
	}
	decoder := json.NewDecoder(http.MaxBytesReader(c.Writer, c.Request.Body, 4096))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&input); err != nil {
		writeAPIError(c, http.StatusBadRequest, "invalid API service speed", "invalid_request")
		return
	}
	if err := decoder.Decode(new(any)); err != io.EOF || input.Revision == 0 || (input.Speed != "standard" && input.Speed != "fast" && input.Speed != "ultrafast") {
		writeAPIError(c, http.StatusBadRequest, "invalid API service speed", "invalid_request")
		return
	}
	state := s.policy.serviceTierDefault
	tier := ""
	if input.Speed == "fast" {
		tier = "priority"
	} else if input.Speed == "ultrafast" {
		tier = "ultrafast"
	}
	state.mu.Lock()
	stale := input.Revision < state.revision || (input.Revision == state.revision && state.tier() != tier)
	if !stale {
		state.value.Store(tier)
		state.revision = input.Revision
	}
	state.mu.Unlock()
	if stale {
		writeAPIError(c, http.StatusConflict, "a newer API service speed is already applied", "stale_speed_update")
		return
	}
	c.JSON(http.StatusOK, gin.H{"speed": input.Speed, "revision": input.Revision})
}
