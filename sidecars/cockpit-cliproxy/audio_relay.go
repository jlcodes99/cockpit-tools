package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"mime"
	"mime/multipart"
	"net/http"
	"net/textproto"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
)

const maxAudioRequestBytes = 26 << 20
const maxAudioResponseBytes = 4 << 20
const audioRequestTimeout = 2 * time.Minute
const defaultTranscriptionModel = "gpt-4o-mini-transcribe"

type audioPart struct {
	header textproto.MIMEHeader
	name   string
	body   []byte
}

type audioRequest struct {
	body        []byte
	contentType string
	operation   string
	model       string
	format      string
	native      bool
	parts       []audioPart
}

func parseAudioRequest(r *http.Request) (audioRequest, string) {
	a := audioRequest{contentType: r.Header.Get("Content-Type"), format: "json"}
	a.native = strings.HasSuffix(r.URL.Path, "/transcribe")
	a.operation = "transcriptions"
	if !a.native {
		a.operation = r.URL.Path[strings.LastIndex(r.URL.Path, "/")+1:]
	}
	body, err := io.ReadAll(io.LimitReader(r.Body, maxAudioRequestBytes+1))
	if err != nil {
		return a, "body_read_failed"
	}
	if len(body) > maxAudioRequestBytes {
		return a, "audio_too_large"
	}
	a.body = body
	mediaType, params, err := mime.ParseMediaType(a.contentType)
	if err != nil {
		return a, "invalid_content_type"
	}
	if a.operation == "speech" {
		if mediaType != "application/json" || !json.Valid(body) {
			return a, "json_required"
		}
		a.model = requestBodyModel(body)
		if a.model == "" {
			return a, "model_required"
		}
		return a, ""
	}
	if mediaType != "multipart/form-data" || params["boundary"] == "" {
		return a, "multipart_required"
	}
	reader := multipart.NewReader(bytes.NewReader(body), params["boundary"])
	fileCount := 0
	seen := make(map[string]bool)
	for {
		part, errPart := reader.NextPart()
		if errPart == io.EOF {
			break
		}
		if errPart != nil {
			return a, "invalid_multipart"
		}
		name := part.FormName()
		if len(a.parts) >= 64 {
			_ = part.Close()
			return a, "too_many_fields"
		}
		payload, errRead := io.ReadAll(part)
		_ = part.Close()
		if errRead != nil {
			return a, "invalid_multipart"
		}
		if name == "file" {
			fileCount++
			if len(payload) == 0 {
				return a, "file_required"
			}
		} else if name == "" || part.FileName() != "" || len(payload) > 64<<10 {
			return a, "invalid_field"
		}
		if name == "model" || name == "response_format" || name == "stream" || name == "language" {
			if seen[name] {
				return a, "duplicate_" + name
			}
			seen[name] = true
		}
		switch name {
		case "model":
			a.model = strings.TrimSpace(string(payload))
		case "response_format":
			a.format = strings.TrimSpace(string(payload))
		}
		a.parts = append(a.parts, audioPart{header: part.Header, name: name, body: payload})
	}
	if fileCount != 1 {
		return a, "one_file_required"
	}
	return a, ""
}

func (a *audioRequest) rewriteMultipart(model string, oauth bool) string {
	var out bytes.Buffer
	writer := multipart.NewWriter(&out)
	for _, part := range a.parts {
		if oauth {
			switch part.name {
			case "model", "response_format":
				continue
			case "stream":
				if strings.TrimSpace(string(part.body)) != "false" {
					return "stream_not_supported"
				}
				continue
			case "file", "language":
			default:
				return "unsupported_" + part.name
			}
		} else if part.name == "model" {
			continue
		}
		target, err := writer.CreatePart(part.header)
		if err != nil {
			return "invalid_multipart"
		}
		if _, err = target.Write(part.body); err != nil {
			return "invalid_multipart"
		}
	}
	if !oauth {
		if err := writer.WriteField("model", model); err != nil {
			return "invalid_multipart"
		}
	}
	if err := writer.Close(); err != nil {
		return "invalid_multipart"
	}
	a.body = out.Bytes()
	a.contentType = writer.FormDataContentType()
	return ""
}

func (s *relayServer) audioError(c *gin.Context, status int, code, detail string) {
	message := s.providerGatewayErrorMessage(code, 0)
	message = strings.ReplaceAll(message, "{{detail}}", detail)
	writeAPIError(c, status, message, code)
}

func (s *relayServer) handleAudio(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	originalRequest := c.Request
	ctx, cancel := context.WithTimeout(relayContext(c), audioRequestTimeout)
	defer cancel()
	c.Request = c.Request.WithContext(ctx)
	defer func() { c.Request = originalRequest }()
	controller := http.NewResponseController(c.Writer)
	_ = controller.SetReadDeadline(time.Now().Add(audioRequestTimeout))
	a, detail := parseAudioRequest(c.Request)
	_ = controller.SetReadDeadline(time.Time{})
	if detail != "" {
		status, code := http.StatusBadRequest, "audio_invalid_request"
		if detail == "audio_too_large" {
			status, code = http.StatusRequestEntityTooLarge, detail
		}
		s.audioError(c, status, code, detail)
		return
	}
	resp, oauth, err := s.forwardAudio(c, spec, &a)
	if err != nil {
		if errAudio, ok := err.(*audioRelayError); ok {
			s.audioError(c, errAudio.status, errAudio.code, errAudio.detail)
		} else {
			s.writeExecutorError(c, err)
		}
		return
	}
	if resp == nil {
		return // Admission failure already wrote an error or the client canceled.
	}
	defer resp.Body.Close()
	observeDirectGatewayResponse(ctx, resp)
	writeUpstreamHeaders(c.Writer.Header(), resp.Header)
	if oauth && !a.native && resp.StatusCode >= 200 && resp.StatusCode < 300 {
		// The rendered transcript determines the converted response's content type.
		c.Writer.Header().Del("Content-Type")
		payload, errRead := io.ReadAll(io.LimitReader(resp.Body, maxAudioResponseBytes+1))
		var transcript struct {
			Text *string `json:"text"`
		}
		if errRead != nil || len(payload) > maxAudioResponseBytes || json.Unmarshal(payload, &transcript) != nil || transcript.Text == nil {
			s.audioError(c, http.StatusBadGateway, "audio_upstream_failed", "")
			return
		}
		if a.format == "text" {
			c.Data(resp.StatusCode, "text/plain; charset=utf-8", []byte(*transcript.Text))
		} else {
			c.JSON(resp.StatusCode, transcript)
		}
		return
	}
	// Preserve binary speech, subtitles, SSE and upstream errors without buffering.
	c.Status(resp.StatusCode)
	buffer := make([]byte, 32<<10)
	for {
		n, errRead := resp.Body.Read(buffer)
		if n > 0 {
			if _, errWrite := c.Writer.Write(buffer[:n]); errWrite != nil {
				return
			}
			c.Writer.Flush()
		}
		if errRead != nil {
			if errRead != io.EOF {
				_ = c.Error(errRead)
			}
			return
		}
	}
}
