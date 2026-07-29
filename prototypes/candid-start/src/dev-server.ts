// A local stand-in for the IC HTTP gateway, on node:http and nothing else.
//
// It forwards every request to `app.httpRequest` and, when the response
// carries the `upgrade` flag, re-dispatches the same request through
// `app.httpRequestUpdate` — the exact query/update dance the real gateway
// performs, which is what makes locally observed behavior meaningful.
import { createServer, type Server } from "node:http";
import type { App, HttpRequest } from "./canister.ts";

export interface DevServerOptions {
  readonly port?: number;
  readonly host?: string;
  /** Request log observer; defaults to silence to keep tests quiet. */
  readonly onRequest?: (line: string) => void;
}

export interface DevServerHandle {
  readonly port: number;
  close(): Promise<void>;
}

export function startDevServer(
  app: App,
  options: DevServerOptions = {},
): Promise<DevServerHandle> {
  const host = options.host ?? "127.0.0.1";
  const onRequest = options.onRequest ?? (() => {});

  const server: Server = createServer((incoming, outgoing) => {
    const chunks: Buffer[] = [];
    incoming.on("data", (chunk: Buffer) => {
      chunks.push(chunk);
    });
    incoming.on("end", () => {
      const headers: (readonly [string, string])[] = [];
      for (const [name, value] of Object.entries(incoming.headers)) {
        if (typeof value === "string") {
          headers.push([name, value]);
        } else if (Array.isArray(value)) {
          for (const each of value) {
            headers.push([name, each]);
          }
        }
      }
      const request: HttpRequest = {
        method: incoming.method ?? "GET",
        url: incoming.url ?? "/",
        headers,
        body: new Uint8Array(Buffer.concat(chunks)),
      };
      void (async () => {
        let response = await app.httpRequest(request);
        if (response.upgrade === true) {
          response = await app.httpRequestUpdate(request);
        }
        onRequest(`${request.method} ${request.url} -> ${response.status}`);
        outgoing.writeHead(
          response.status,
          Object.fromEntries(response.headers),
        );
        outgoing.end(Buffer.from(response.body));
      })().catch(() => {
        outgoing.writeHead(500, { "content-type": "text/plain" });
        outgoing.end("dev server failure");
      });
    });
  });

  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(options.port ?? 0, host, () => {
      const address = server.address();
      const port =
        address !== null && typeof address === "object" ? address.port : 0;
      resolve({
        port,
        close: () =>
          new Promise<void>((done, fail) => {
            server.close((error) => (error ? fail(error) : done()));
          }),
      });
    });
  });
}
