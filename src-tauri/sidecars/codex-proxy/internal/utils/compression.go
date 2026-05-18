package utils

import (
	"bytes"
	"compress/gzip"
	"io"
	"log"
	"net/http"
)

// DecompressGzipIfNeeded 检测并解压缩 gzip 响应体
// 这是一个兜底机制，用于处理错误响应等特殊场景
// 正常情况下，Go 的 http.Client 会自动处理 gzip 解压缩
func DecompressGzipIfNeeded(resp *http.Response, bodyBytes []byte) []byte {
	// 检查 Content-Encoding 头
	if resp.Header.Get("Content-Encoding") != "gzip" {
		return bodyBytes
	}

	// 尝试解压缩
	reader, err := gzip.NewReader(bytes.NewReader(bodyBytes))
	if err != nil {
		log.Printf("[Gzip] 警告: 创建 gzip reader 失败: %v", err)
		return bodyBytes
	}
	defer reader.Close()

	decompressed, err := io.ReadAll(reader)
	if err != nil {
		log.Printf("[Gzip] 警告: 解压缩 gzip 响应体失败: %v", err)
		return bodyBytes
	}

	return decompressed
}
