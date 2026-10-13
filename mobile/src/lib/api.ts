import type { Pairing } from './pair';
import type { Hello, SessionDetail, SessionList } from './types';

export class BridgeError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

const TIMEOUT_MS = 10_000;

/** Talks to one paired computer. Remembers ETags, so polling an unchanged list is cheap. */
export class Bridge {
  private tags = new Map<string, { tag: string; body: unknown }>();

  constructor(readonly pairing: Pairing) {}

  private async request(method: 'GET' | 'POST', path: string, body?: unknown): Promise<unknown> {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.pairing.token}`,
      Accept: 'application/json',
    };
    const cached = method === 'GET' ? this.tags.get(path) : undefined;
    if (cached) headers['If-None-Match'] = cached.tag;
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
    let response: Response;
    try {
      response = await fetch(`${this.pairing.url}${path}`, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: controller.signal,
      });
    } catch {
      throw new BridgeError(
        `Can't reach ${this.pairing.url}. Check that \`runo --phone\` is running and this phone is on the same network or Tailscale.`,
        0,
      );
    } finally {
      clearTimeout(timer);
    }
    if (response.status === 304 && cached) {
      return cached.body;
    }
    const text = await response.text();
    let json: unknown = null;
    try {
      json = text ? JSON.parse(text) : null;
    } catch {
      throw new BridgeError('The computer sent something that isn’t Runo’s bridge.', response.status);
    }
    if (!response.ok) {
      const error = (json as { error?: string } | null)?.error;
      throw new BridgeError(error ?? `Request failed (${response.status}).`, response.status);
    }
    const tag = response.headers.get('etag');
    if (method === 'GET' && tag) this.tags.set(path, { tag, body: json });
    return json;
  }

  hello() {
    return this.request('GET', '/v1/hello') as Promise<Hello>;
  }

  sessions() {
    return this.request('GET', '/v1/sessions') as Promise<SessionList>;
  }

  session(id: number) {
    return this.request('GET', `/v1/sessions/${id}`) as Promise<SessionDetail>;
  }

  prompt(id: number, text: string) {
    return this.request('POST', `/v1/sessions/${id}/prompt`, { text });
  }

  stop(id: number) {
    return this.request('POST', `/v1/sessions/${id}/stop`);
  }

  approvePlan(id: number) {
    return this.request('POST', `/v1/sessions/${id}/plan`);
  }

  allow(id: number, tools: string[]) {
    return this.request('POST', `/v1/sessions/${id}/allow`, { tools });
  }

  answer(id: number, picks: (string | null)[]) {
    return this.request('POST', `/v1/sessions/${id}/answers`, { picks });
  }

  approval(requestId: string, allow: boolean) {
    return this.request('POST', `/v1/approvals/${encodeURIComponent(requestId)}`, { allow });
  }
}
