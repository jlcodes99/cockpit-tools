import { previewDurableUiSetting, readDurableUiSetting } from './durableUiSettings';

const PRIVACY_MODE_STORAGE_KEY = 'agtools.privacy_mode_enabled'
export const PRIVACY_MODE_CHANGED_EVENT = 'agtools:privacy-mode-changed'

function maskSegment(value: string, keepStart = 2, keepEnd = 2): string {
  const raw = value.trim()
  if (!raw) return raw
  if (raw.length <= 2) return `${raw.charAt(0)}*`
  if (raw.length <= keepStart + keepEnd) return `${raw.slice(0, 1)}***${raw.slice(-1)}`
  return `${raw.slice(0, keepStart)}***${raw.slice(-keepEnd)}`
}

function maskEmail(value: string): string {
  const [localPart = '', domainPart = ''] = value.split('@')
  const localMasked = maskSegment(localPart, 2, 1)
  if (!domainPart) return `${localMasked}@***`

  const domainTokens = domainPart.split('.').filter(Boolean)
  if (domainTokens.length === 0) return `${localMasked}@***`

  if (domainTokens.length === 1) {
    return `${localMasked}@${maskSegment(domainTokens[0], 1, 1)}`
  }

  const tld = domainTokens[domainTokens.length - 1]
  const host = domainTokens.slice(0, -1).map((item) => maskSegment(item, 1, 1)).join('.')
  return `${localMasked}@${host}.${tld}`
}

function maskGeneric(value: string): string {
  const raw = value.trim()
  if (!raw) return raw
  if (raw.length <= 3) return `${raw.charAt(0)}**`
  if (raw.length <= 6) return `${raw.slice(0, 1)}***${raw.slice(-1)}`
  if (raw.length <= 10) return `${raw.slice(0, 2)}***${raw.slice(-2)}`
  return `${raw.slice(0, 3)}***${raw.slice(-3)}`
}

export function isPrivacyModeEnabledByDefault(): boolean {
  try {
    return readDurableUiSetting(PRIVACY_MODE_STORAGE_KEY) === '1'
  } catch {
    return false
  }
}

export function persistPrivacyModeEnabled(enabled: boolean): void {
  previewDurableUiSetting(PRIVACY_MODE_STORAGE_KEY, enabled ? '1' : '0')
}

export function maskSensitiveValue(value: string | null | undefined, enabled: boolean): string {
  const raw = (value ?? '').trim()
  if (!raw || !enabled) return raw
  if (raw.includes('@')) return maskEmail(raw)
  return maskGeneric(raw)
}
