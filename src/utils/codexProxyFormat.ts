/**
 * 代理工作台的纯格式化工具。
 * 字节口径与代理活动面板保持一致（1024 进制，B / KB / MB），
 * 这样连接表和活动面板可以用同一种读法展示同一个字节数。
 */

/** 缺失或无法解析的值的统一占位符。 */
export const PROXY_UNKNOWN = '—';

function nonNegative(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function pad(value: number): string {
  return String(value).padStart(2, '0');
}

/** B / KB / MB，1024 进制；非法值按 0 处理，避免出现 `NaN B`。 */
export function formatProxyBytes(bytes: number): string {
  const value = nonNegative(bytes);
  if (value < 1024) return `${Math.round(value)} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

/** 速率复用同一套字节单位，只追加每秒后缀。 */
export function formatProxyRate(bytesPerSecond: number): string {
  return `${formatProxyBytes(bytesPerSecond)}/s`;
}

/** 单条连接的持续时长：`12s`、`1m 05s`、`2h 03m`、`1d 04h`。 */
export function formatProxyDuration(milliseconds: number): string {
  const totalSeconds = Math.floor(nonNegative(milliseconds) / 1000);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600) % 24;
  const days = Math.floor(totalSeconds / 86400);
  if (days > 0) return `${days}d ${pad(hours)}h`;
  if (hours > 0) return `${hours}h ${pad(minutes)}m`;
  if (minutes > 0) return `${minutes}m ${pad(seconds)}s`;
  return `${seconds}s`;
}

/** 把 ISO 字符串或毫秒时间戳统一成毫秒；无法解析时返回 null，绝不返回 `NaN`。 */
export function proxyTimeMs(value: string | number | null | undefined): number | null {
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  if (typeof value !== 'string' || !value.trim()) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

/** 本机时区的时分秒，用于连接开始时间。 */
export function formatProxyClock(value: string | number | null | undefined): string {
  const at = proxyTimeMs(value);
  if (at === null) return PROXY_UNKNOWN;
  const date = new Date(at);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** 本机时区的 `YYYY-MM-DD HH:mm`，用于来源更新时间与到期时间。 */
export function formatProxyDateTime(value: string | number | null | undefined): string {
  const at = proxyTimeMs(value);
  if (at === null) return PROXY_UNKNOWN;
  const date = new Date(at);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/**
 * 用量进度：0–100 之间的一位小数。
 * 总量缺失、为 0 或异常时返回 0，让调用方隐藏进度条而不是画出假进度。
 */
export function proxyUsagePercent(used: number, total: number): number {
  const limit = nonNegative(total);
  if (limit <= 0) return 0;
  const ratio = (nonNegative(used) / limit) * 100;
  return Math.min(100, Math.round(ratio * 10) / 10);
}
