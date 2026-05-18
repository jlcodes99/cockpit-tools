package images

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"mime/multipart"
	"net/textproto"
	"strings"

	"github.com/gin-gonic/gin"
)

func isMultipartContentType(contentType string) bool {
	mediaType, _, err := mime.ParseMediaType(contentType)
	return err == nil && strings.EqualFold(mediaType, "multipart/form-data")
}

func isJSONContentType(contentType string) bool {
	mediaType, _, err := mime.ParseMediaType(contentType)
	if err != nil {
		return strings.Contains(strings.ToLower(contentType), "application/json")
	}
	return strings.EqualFold(mediaType, "application/json")
}

func extractImagesModel(bodyBytes []byte, contentType string) string {
	if isMultipartContentType(contentType) {
		value, _ := extractMultipartField(bodyBytes, contentType, "model")
		return value
	}

	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return ""
	}
	model, _ := reqMap["model"].(string)
	return model
}

func isImagesStreamRequest(c *gin.Context, bodyBytes []byte, contentType string) bool {
	if strings.EqualFold(c.Query("stream"), "true") {
		return true
	}
	if isMultipartContentType(contentType) {
		value, ok := extractMultipartField(bodyBytes, contentType, "stream")
		return ok && strings.EqualFold(value, "true")
	}

	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return false
	}
	return jsonValueIsTrue(reqMap["stream"])
}

func jsonValueIsTrue(value interface{}) bool {
	switch v := value.(type) {
	case bool:
		return v
	case string:
		return strings.EqualFold(v, "true")
	default:
		return false
	}
}

type multipartDiagnosticError struct {
	stage  string
	reason string
	err    error
}

func (e *multipartDiagnosticError) Error() string {
	if e == nil {
		return ""
	}
	if e.err == nil {
		return e.reason
	}
	return e.err.Error()
}

func (e *multipartDiagnosticError) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.err
}

func newMultipartDiagnosticError(stage string, reason string, err error) error {
	return &multipartDiagnosticError{
		stage:  stage,
		reason: reason,
		err:    err,
	}
}

func describeMultipartDiagnostic(err error) (string, string) {
	if err == nil {
		return "", ""
	}
	var diagErr *multipartDiagnosticError
	if errors.As(err, &diagErr) {
		return diagErr.stage, diagErr.reason
	}
	if errors.Is(err, io.ErrUnexpectedEOF) || strings.Contains(strings.ToLower(err.Error()), "unexpected eof") {
		return "read_part", "unexpected_eof"
	}
	return "read_part", "part_read_failed"
}

func hasMultipartBoundary(contentType string) bool {
	_, params, err := mime.ParseMediaType(contentType)
	if err != nil {
		return false
	}
	return strings.TrimSpace(params["boundary"]) != ""
}

func extractMultipartField(bodyBytes []byte, contentType string, fieldName string) (string, bool) {
	reader, err := newMultipartReader(bodyBytes, contentType)
	if err != nil {
		return "", false
	}

	for {
		part, err := reader.NextPart()
		if err == io.EOF {
			return "", false
		}
		if err != nil {
			return "", false
		}
		if part.FormName() != fieldName || part.FileName() != "" {
			part.Close()
			continue
		}
		valueBytes, err := io.ReadAll(part)
		part.Close()
		if err != nil {
			return "", false
		}
		return string(valueBytes), true
	}
}

func validateMultipartBody(bodyBytes []byte, contentType string) error {
	reader, err := newMultipartReader(bodyBytes, contentType)
	if err != nil {
		return err
	}
	for {
		part, err := reader.NextPart()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			reason := "malformed_multipart"
			if errors.Is(err, io.ErrUnexpectedEOF) || strings.Contains(strings.ToLower(err.Error()), "unexpected eof") {
				reason = "unexpected_eof"
			}
			return newMultipartDiagnosticError("next_part", reason, err)
		}
		_, readErr := io.Copy(io.Discard, part)
		part.Close()
		if readErr != nil {
			reason := "part_read_failed"
			if errors.Is(readErr, io.ErrUnexpectedEOF) || strings.Contains(strings.ToLower(readErr.Error()), "unexpected eof") {
				reason = "unexpected_eof"
			}
			return newMultipartDiagnosticError("read_part", reason, readErr)
		}
	}
}

func rewriteMultipartFormField(bodyBytes []byte, contentType string, fieldName string, fieldValue string) ([]byte, string, error) {
	reader, err := newMultipartReader(bodyBytes, contentType)
	if err != nil {
		return nil, "", err
	}

	var buf bytes.Buffer
	writer := multipart.NewWriter(&buf)
	fieldWritten := false

	for {
		part, err := reader.NextPart()
		if err == io.EOF {
			break
		}
		if err != nil {
			reason := "malformed_multipart"
			if errors.Is(err, io.ErrUnexpectedEOF) || strings.Contains(strings.ToLower(err.Error()), "unexpected eof") {
				reason = "unexpected_eof"
			}
			return nil, "", newMultipartDiagnosticError("rewrite_field", reason, err)
		}

		formName := part.FormName()
		fileName := part.FileName()
		if formName == fieldName && fileName == "" {
			if !fieldWritten {
				if err := writer.WriteField(fieldName, fieldValue); err != nil {
					part.Close()
					return nil, "", newMultipartDiagnosticError("rewrite_field", "part_read_failed", err)
				}
				fieldWritten = true
			}
			part.Close()
			continue
		}

		if err := copyMultipartPart(writer, part); err != nil {
			part.Close()
			reason := "part_read_failed"
			if errors.Is(err, io.ErrUnexpectedEOF) || strings.Contains(strings.ToLower(err.Error()), "unexpected eof") {
				reason = "unexpected_eof"
			}
			return nil, "", newMultipartDiagnosticError("rewrite_field", reason, err)
		}
		part.Close()
	}

	if !fieldWritten {
		if err := writer.WriteField(fieldName, fieldValue); err != nil {
			return nil, "", newMultipartDiagnosticError("rewrite_field", "part_read_failed", err)
		}
	}
	if err := writer.Close(); err != nil {
		return nil, "", newMultipartDiagnosticError("rewrite_field", "part_read_failed", err)
	}

	return buf.Bytes(), writer.FormDataContentType(), nil
}

func newMultipartReader(bodyBytes []byte, contentType string) (*multipart.Reader, error) {
	_, params, err := mime.ParseMediaType(contentType)
	if err != nil {
		return nil, newMultipartDiagnosticError("parse_media_type", "invalid_content_type", err)
	}
	boundary := params["boundary"]
	if boundary == "" {
		return nil, newMultipartDiagnosticError("missing_boundary", "missing_boundary", fmt.Errorf("missing multipart boundary"))
	}
	return multipart.NewReader(bytes.NewReader(bodyBytes), boundary), nil
}

func copyMultipartPart(writer *multipart.Writer, part *multipart.Part) error {
	header := textproto.MIMEHeader{}
	for key, values := range part.Header {
		for _, value := range values {
			header.Add(key, value)
		}
	}
	newPart, err := writer.CreatePart(header)
	if err != nil {
		return err
	}
	_, err = io.Copy(newPart, part)
	return err
}
