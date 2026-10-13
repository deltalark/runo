// Pairing links printed by `runo --phone`: runo://pair?url=http%3A%2F%2F100.64.0.2%3A7631&token=…
// Kept free of React Native imports so it runs under `node --test`.

export interface Pairing {
  url: string;
  token: string;
}

/** The bridge address without a trailing slash, with http:// added when it was left out. */
export function normalizeUrl(raw: string): string {
  let url = raw.trim().replace(/\/+$/, '');
  if (url && !/^https?:\/\//i.test(url)) {
    url = `http://${url}`;
  }
  return url;
}

/** Read a pasted or scanned pairing link. Returns null when it isn't one. */
export function parsePairLink(text: string): Pairing | null {
  const trimmed = text.trim();
  const query = trimmed.match(/^runo:\/\/\/?pair\?(.*)$/i)?.[1];
  if (query === undefined) {
    return null;
  }
  const params = new Map<string, string>();
  for (const part of query.split('&')) {
    const eq = part.indexOf('=');
    if (eq < 0) continue;
    try {
      params.set(part.slice(0, eq), decodeURIComponent(part.slice(eq + 1)));
    } catch {
      return null;
    }
  }
  const url = normalizeUrl(params.get('url') ?? '');
  const token = (params.get('token') ?? '').trim();
  if (!url || !token) {
    return null;
  }
  return { url, token };
}
