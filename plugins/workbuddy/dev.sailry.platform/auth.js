// SPDX-License-Identifier: MIT
// Token field spellings follow the fixed reference listed in NOTICE.
export function exchange(document) {
  const data = document?.data;
  if (!data || typeof data !== 'object') throw new Error('Invalid WorkBuddy token response');
  const text = names => names.map(name => data[name]).find(value => typeof value === 'string' && value);
  const access = text(['accessToken', 'access_token', 'AccessToken']);
  const refresh = text(['refreshToken', 'refresh_token', 'RefreshToken']);
  const domain = text(['domain', 'Domain']);
  let expires = data.expiresIn ?? data.expires_in;
  if (expires == null) {
    const value = data.expiresAt ?? data.expires_at;
    if (typeof value === 'number' && Number.isFinite(value)) {
      const millis = value > 1e12 ? value : value * 1000;
      expires = Math.floor((millis - Date.now()) / 1000);
    }
  }
  if (!access || !Number.isSafeInteger(expires) || expires <= 0) throw new Error('Invalid WorkBuddy token lifetime');
  return {access, ...(refresh ? {refresh} : {}), expires_in: expires, ...(domain ? {domain} : {})};
}
