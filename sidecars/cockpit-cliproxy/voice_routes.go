package main

import "github.com/gin-gonic/gin"

func (s *relayServer) registerVoiceAliases(router *gin.Engine) {
	// Native Codex clients append these paths to chatgpt_base_url.
	router.POST("/backend-api/codex/realtime/calls", s.handleCodexLive)
	router.GET("/backend-api/codex/:call_id", s.handleCodexLiveSideband)
	router.GET("/backend-api/codex/realtime/calls/:call_id", s.handleCodexLiveSideband)
	for _, path := range []string{"/transcribe", "/v1/transcribe", "/backend-api/transcribe", "/backend-api/codex/transcribe"} {
		router.POST(path, s.handleAudio)
	}
	for _, prefix := range []string{"/v1/audio", "/audio"} {
		for _, operation := range []string{"transcriptions", "translations", "speech"} {
			router.POST(prefix+"/"+operation, s.handleAudio)
		}
	}
}

func isAudioPath(path string) bool {
	switch path {
	case "/transcribe", "/v1/transcribe", "/backend-api/transcribe", "/backend-api/codex/transcribe",
		"/v1/audio/transcriptions", "/audio/transcriptions", "/v1/audio/translations", "/audio/translations",
		"/v1/audio/speech", "/audio/speech":
		return true
	default:
		return false
	}
}
