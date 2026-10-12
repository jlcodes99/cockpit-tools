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
	fast     atomic.Bool
	mu       sync.Mutex
	revision uint64
}

func newAPIServiceTierDefault(tier string) *apiServiceTierDefault {
	state := &apiServiceTierDefault{}
	state.fast.Store(normalizedUsageServiceTier(tier) == "priority")
	return state
}

func (s *apiServiceTierDefault) tier() string {
	if s.fast.Load() {
		return "priority"
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
	if err := decoder.Decode(new(any)); err != io.EOF || input.Revision == 0 || (input.Speed != "standard" && input.Speed != "fast") {
		writeAPIError(c, http.StatusBadRequest, "invalid API service speed", "invalid_request")
		return
	}
	state := s.policy.serviceTierDefault
	fast := input.Speed == "fast"
	state.mu.Lock()
	stale := input.Revision < state.revision || (input.Revision == state.revision && state.fast.Load() != fast)
	if !stale {
		state.fast.Store(fast)
		state.revision = input.Revision
	}
	state.mu.Unlock()
	if stale {
		writeAPIError(c, http.StatusConflict, "a newer API service speed is already applied", "stale_speed_update")
		return
	}
	c.JSON(http.StatusOK, gin.H{"speed": input.Speed, "revision": input.Revision})
}
