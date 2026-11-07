import { invoke } from '@tauri-apps/api/core'

// 缓存的系统真实时区偏移（分钟，相对 UTC）。WebView 的 Intl 默认时区在部分环境会
// 解析成 UTC，且部分 webview 的 ICU 时区库无法解析 IANA 名称而再次静默回退 UTC，
// 导致时间偏移 N 小时。因此由 Rust 侧用 chrono::Local 读取 OS 真实偏移（与日志
// 本地时间一致），前端以 `UTC + offset` 方式渲染，彻底绕开 webview 时区库。
let cachedOffsetMinutes: number | null = null

let initPromise: Promise<void> | null = null

// 在应用启动早期调用一次，从后端拉取真实时区偏移。失败则保持 null，
// getLocalOffsetMinutes 回退到 webview 自带偏移（最坏情况为 UTC）。
export function initLocalTimeZone(): Promise<void> {
  if (initPromise) return initPromise
  initPromise = (async () => {
    try {
      cachedOffsetMinutes = await invoke<number>('get_local_timezone')
    } catch {
      cachedOffsetMinutes = null
    }
  })()
  return initPromise
}

// 同步读取本地时区偏移（分钟）。供格式化时把 UTC 时间戳平移到本地墙上时间。
export function getLocalOffsetMinutes(): number {
  if (cachedOffsetMinutes != null) return cachedOffsetMinutes
  // 回退：webview 自带偏移（本地=UTC 时为 0）。
  return -new Date().getTimezoneOffset()
}
