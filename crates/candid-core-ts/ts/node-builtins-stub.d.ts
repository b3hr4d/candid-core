// Hermetic stand-ins for the Node built-ins the runtime tests use, in the
// same spirit as principal-stub.d.ts: the type-check needs no npm package
// beyond the compiler itself, and `@types/node` would be a new supply-chain
// surface for the sake of four signatures. Only what the tests call is
// declared; widening these is a deliberate act, not a convenience.

declare module "node:test" {
  export function test(
    name: string,
    fn: () => void | Promise<void>,
  ): void;
}

declare module "node:assert/strict" {
  interface Assert {
    (value: unknown, message?: string): void;
    strictEqual(actual: unknown, expected: unknown, message?: string): void;
    deepStrictEqual(actual: unknown, expected: unknown, message?: string): void;
  }
  const assert: Assert;
  export default assert;
}

declare module "node:fs" {
  export function readFileSync(path: URL | string, encoding: "utf8"): string;
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
