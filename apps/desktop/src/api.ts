// One way to reach the agent: through Electron's IPC bridge in the app, or the dev proxy in a browser.

type BridgeResponse = { status: number; body?: unknown; bytes?: Uint8Array; type?: string };
type Bridge = {
  request(method: string, path: string, body: unknown, headers?: Record<string, string>): Promise<BridgeResponse>;
  onEvent(fn: (data: string) => void): () => void;
  saveDialog(name: string): Promise<string | null>;
  setContentProtection(on: boolean): Promise<void>;
};

declare global {
  interface Window {
    sotto?: Bridge;
    __SPLASH__?: { hide(): void };
  }
}

const bridge = typeof window !== "undefined" ? window.sotto : undefined;

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

export async function api<T = unknown>(method: string, path: string, body?: unknown, headers?: Record<string, string>): Promise<T> {
  let status: number;
  let data: unknown;
  if (bridge) {
    const r = await bridge.request(method, path, body ?? null, headers);
    status = r.status;
    data = r.body;
  } else {
    const bytes = body instanceof Uint8Array;
    const res = await fetch(path, {
      method,
      headers: { ...(body != null && !bytes ? { "content-type": "application/json" } : {}), ...headers },
      body: body == null ? undefined : bytes ? (body as Uint8Array<ArrayBuffer>) : JSON.stringify(body),
    }).catch(() => null);
    if (!res) throw new ApiError("Sotto's background service isn't responding.", 503);
    status = res.status;
    data = await res.json().catch(() => null);
  }
  if (status >= 400) {
    const message = (data as { error?: string } | null)?.error ?? "Something went wrong.";
    throw new ApiError(message, status);
  }
  return data as T;
}

export const get = <T>(path: string) => api<T>("GET", path);
export const post = <T>(path: string, body?: unknown) => api<T>("POST", path, body ?? {});
export const patch = <T>(path: string, body: unknown) => api<T>("PATCH", path, body);
export const del = <T>(path: string) => api<T>("DELETE", path);

/** Fetch an image (review page preview) as an object URL. */
export async function imageUrl(path: string): Promise<string> {
  if (bridge) {
    const r = await bridge.request("GET", path, null);
    if (r.status >= 400 || !r.bytes) throw new ApiError("Couldn't load this page.", r.status);
    return URL.createObjectURL(new Blob([r.bytes as Uint8Array<ArrayBuffer>], { type: r.type ?? "image/png" }));
  }
  const res = await fetch(path);
  if (!res.ok) throw new ApiError("Couldn't load this page.", res.status);
  return URL.createObjectURL(await res.blob());
}

export type AgentEvent = { type: string; [k: string]: unknown };

/** Live updates from the agent. Reconnects with backoff in the browser; Electron forwards its own stream. */
export function subscribe(onEvent: (e: AgentEvent) => void): () => void {
  const parse = (raw: string) => {
    try {
      onEvent(JSON.parse(raw) as AgentEvent);
    } catch {
      /* ignore malformed lines */
    }
  };
  if (bridge) return bridge.onEvent(parse);
  let stopped = false;
  let ctrl: AbortController | null = null;
  let delay = 1000;
  (async () => {
    while (!stopped) {
      try {
        ctrl = new AbortController();
        const res = await fetch("/api/events", { signal: ctrl.signal });
        if (!res.ok || !res.body) throw new Error("no stream");
        const reader = res.body.getReader();
        const dec = new TextDecoder();
        let buf = "";
        delay = 1000;
        onEvent({ type: "connected" });
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          let i;
          while ((i = buf.indexOf("\n\n")) >= 0) {
            const line = buf
              .slice(0, i)
              .split("\n")
              .find((l) => l.startsWith("data:"));
            buf = buf.slice(i + 2);
            if (line) parse(line.slice(5).trim());
          }
        }
      } catch {
        /* reconnect below */
      }
      if (stopped) break;
      onEvent({ type: "disconnected" });
      await new Promise((r) => setTimeout(r, delay + Math.random() * 500));
      delay = Math.min(delay * 2, 30_000);
    }
  })();
  return () => {
    stopped = true;
    ctrl?.abort();
  };
}

/** Ask where to save a file. In the app this is the system save dialog. */
export async function chooseSavePath(name: string): Promise<string | null> {
  if (bridge) return bridge.saveDialog(name);
  return window.prompt("Development build: full path to save the clean copy", `/tmp/${name}`);
}

export const inApp = Boolean(bridge);
