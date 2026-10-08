package main

import (
	"crypto/subtle"
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
)

const requestDiagnosticsConfigPath = "/v1/cockpit/diagnostics/config"
const requestDiagnosticsEventsPath = "/v1/cockpit/diagnostics/events"

func isRequestDiagnosticsControlPath(path string) bool {
	return path == requestDiagnosticsConfigPath || path == requestDiagnosticsEventsPath
}

func (m *manifest) payloadLoggingEnabled() bool {
	if m == nil {
		return false
	}
	if value := m.requestPayloadLoggingOverride.Load(); value != nil {
		return *value
	}
	return m.RequestPayloadLogging
}

// The host controls this through its internal key. Applying the switch changes
// future requests without interrupting active streams or restarting a gateway.
func (s *relayServer) handleRequestDiagnosticsConfig(c *gin.Context) {
	if !s.requireDiagnosticsControlKey(c) {
		return
	}
	var input struct {
		RequestPayloadLogging *bool `json:"requestPayloadLogging"`
	}
	if err := c.ShouldBindJSON(&input); err != nil || input.RequestPayloadLogging == nil {
		writeAPIError(c, http.StatusBadRequest, "invalid diagnostic config", "invalid_request")
		return
	}
	s.manifest.requestPayloadLoggingOverride.Store(input.RequestPayloadLogging)
	c.JSON(http.StatusOK, gin.H{"requestPayloadLogging": *input.RequestPayloadLogging})
}

func (s *relayServer) requireDiagnosticsControlKey(c *gin.Context) bool {
	controlKey := ""
	if s.manifest != nil {
		controlKey = strings.TrimSpace(s.manifest.DiagnosticsControlKey)
	}
	if controlKey == "" || subtle.ConstantTimeCompare([]byte(extractClientAPIKey(c.Request)), []byte(controlKey)) != 1 {
		writeAPIError(c, http.StatusForbidden, "diagnostic control key required", "permission_denied")
		return false
	}
	if s.manifest == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "manifest unavailable", "service_unavailable")
		return false
	}
	return true
}

func (s *relayServer) handleRequestDiagnosticsEvents(c *gin.Context) {
	if !s.requireDiagnosticsControlKey(c) {
		return
	}
	events := make([]requestDiagnosticsPayload, 0, 8)
	var dropped uint64
	if s.policy != nil {
		if queue := s.policy.diagnosticsWorker(); queue != nil {
			dropped = queue.dropped.Load()
			for len(events) < 8 {
				select {
				case event := <-queue.ready:
					events = append(events, event)
				default:
					c.JSON(http.StatusOK, gin.H{"events": events, "dropped": dropped})
					return
				}
			}
		}
	}
	c.JSON(http.StatusOK, gin.H{"events": events, "dropped": dropped})
}
