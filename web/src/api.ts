// Talking to the `ordo web` that served this page. The session token rides in
// the URL fragment, which the browser never sends anywhere, and goes back to
// the server as a header (or a query parameter where a header cannot go).

import type { ActResult, FileData, ItemFacts, Slot, Theme } from "./types.ts";

export class Api {
  readonly token: string;
  readonly rev: string;

  constructor(token: string, rev: string) {
    this.token = token;
    this.rev = rev;
  }

  private url(path: string, params: Record<string, string | number> = {}): string {
    const q = new URLSearchParams({ rev: this.rev });
    for (const [k, v] of Object.entries(params)) q.set(k, String(v));
    return `${path}?${q}`;
  }

  private async get<T>(path: string, params: Record<string, string | number> = {}): Promise<T> {
    const res = await fetch(this.url(path, params), { headers: { "X-Ordo-Token": this.token } });
    return answer<T>(res);
  }

  review(): Promise<Slot> {
    return this.get("/api/review");
  }

  file(path: string): Promise<FileData> {
    return this.get("/api/file", { path });
  }

  item(i: number): Promise<ItemFacts> {
    return this.get("/api/item", { i });
  }

  themes(): Promise<{ themes: Record<string, Theme>; chosen: string | null }> {
    return this.get("/api/themes");
  }

  async act(body: Record<string, unknown>): Promise<ActResult> {
    const res = await fetch(this.url("/api/act"), {
      method: "POST",
      headers: { "X-Ordo-Token": this.token, "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    return answer<ActResult>(res);
  }

  events(on: (event: string, data: unknown) => void): EventSource {
    const es = new EventSource(this.url("/api/events", { t: this.token }));
    for (const name of ["progress", "changed"]) {
      es.addEventListener(name, (e) => on(name, JSON.parse((e as MessageEvent).data)));
    }
    return es;
  }
}

/** The body as JSON, or the server's error as one — whatever shape it came in. */
async function answer<T>(res: Response): Promise<T> {
  const text = await res.text();
  let body: { error?: string } | null = null;
  try {
    body = JSON.parse(text);
  } catch {
    // a proxy's or the browser's own error page
  }
  if (!res.ok) throw new Error(body?.error ?? `${res.status} ${res.statusText}`);
  if (body === null) throw new Error("the server sent something that is not JSON");
  return body as T;
}
