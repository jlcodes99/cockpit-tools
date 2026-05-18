package main

// 版本信息变量 - 在构建时通过 -ldflags 注入
// 实际值从根目录 VERSION 文件读取
var (
	// Version 当前版本号（构建时从 VERSION 文件注入）
	Version = "v0.0.0-dev"

	// BuildTime 构建时间（构建时注入）
	BuildTime = "unknown"

	// GitCommit Git提交哈希（构建时注入）
	GitCommit = "unknown"
)
