// Local entry: `npm run dev`, then browse http://127.0.0.1:3000/.
import { startDevServer } from "../../src/dev-server.ts";
import { createNotesApp } from "./app.ts";

const app = createNotesApp();
const port = Number(process.env["PORT"] ?? "3000");

const handle = await startDevServer(app, {
  port,
  onRequest: (line) => console.log(line),
});

console.log(`candid-start notes on http://127.0.0.1:${handle.port}/`);
console.log(`emitted candid interface:\n${app.didText()}`);
