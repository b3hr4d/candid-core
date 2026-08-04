// Hermetic stand-ins for the Node built-ins the runtime tests use, in the
// same spirit as principal-stub.d.ts: the type-check needs no npm package
// beyond the compiler itself, and `@types/node` would be a new supply-chain
// surface for the sake of four signatures. Only what the tests call is
// declared; widening these is a deliberate act, not a convenience.

declare module "node:test" {
  export function test(name: string, fn: () => void | Promise<void>): void;
}

declare module "node:assert/strict" {
  interface Assert {
    (value: unknown, message?: string): void;
    strictEqual(actual: unknown, expected: unknown, message?: string): void;
    // Widened for the actor integration suite (issue #104): promise
    // rejection and synchronous-throw assertions.
    rejects(block: () => Promise<unknown>, check?: (error: unknown) => boolean): Promise<void>;
    throws(block: () => unknown, expected?: unknown): void;
    deepStrictEqual(actual: unknown, expected: unknown, message?: string): void;
  }
  const assert: Assert;
  export default assert;
}

declare module "node:fs" {
  export function readFileSync(path: URL | string, encoding: "utf8"): string;
  // Widened for the wire-vector goldens (issue #103): UPDATE_GOLDENS=1 npm
  // test rewrites the TS-encoder hex files the Rust differential reads back.
  export function writeFileSync(path: URL | string, data: string): void;
}

declare module "node:vm" {
  export function runInNewContext(code: string): unknown;
}

declare module "node:process" {
  // Widened for the wire-vector goldens (issue #103): the codec suite reads
  // UPDATE_GOLDENS to decide whether to rewrite the TS-encoder hex files.
  const process: { readonly env: { readonly [name: string]: string | undefined } };
  export default process;
}

declare module "node:module" {
  interface ResolveResult {
    url: string;
    shortCircuit?: boolean;
  }
  export function registerHooks(hooks: {
    resolve(
      specifier: string,
      context: unknown,
      nextResolve: (specifier: string, context: unknown) => ResolveResult,
    ): ResolveResult;
  }): void;
}
