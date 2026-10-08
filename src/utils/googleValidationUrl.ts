/**
 * 为 Google 验证/登录链接注入目标账号标识（authuser / login_hint / Email），
 * 强制锁定目标账号，避免浏览器中已登录的其他 Google 账号（如默认账号）自动跳过验证或串号。
 */
export function formatGoogleValidationUrl(url: string, email?: string | null): string {
  if (!url || !email) return url
  const trimmedEmail = email.trim()
  if (!trimmedEmail) return url

  try {
    const u = new URL(url)
    const hostname = u.hostname.toLowerCase()
    const isGoogleHost =
      hostname === 'accounts.google.com' ||
      hostname === 'google.com' ||
      hostname.endsWith('.google.com')

    if (!isGoogleHost) {
      return url
    }

    // 若 authuser 为空串、不存在或为数字索引（如 0），设置为目标账号 email
    const existingAuthUser = u.searchParams.get('authuser')
    if (!existingAuthUser || existingAuthUser.trim() === '' || /^\d+$/.test(existingAuthUser.trim())) {
      u.searchParams.set('authuser', trimmedEmail)
    }
    const existingLoginHint = u.searchParams.get('login_hint')
    if (!existingLoginHint || existingLoginHint.trim() === '') {
      u.searchParams.set('login_hint', trimmedEmail)
    }
    const existingEmail = u.searchParams.get('Email')
    if (!existingEmail || existingEmail.trim() === '') {
      u.searchParams.set('Email', trimmedEmail)
    }
    return u.toString()
  } catch {
    return url
  }
}
