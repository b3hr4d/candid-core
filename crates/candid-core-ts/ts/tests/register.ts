// The runtime half of the `@candid-core/schema` placeholder specifier. The
// goldens import it; tsc resolves it through tsconfig `paths`, and Node —
// which reads no tsconfig — resolves it through this hook instead, loaded via
// `--import` in the npm test script. One mapping, declared in the same shape
// in both resolvers, pointing at the same file: the goldens run against
// exactly the schema core they were type-checked against.
import { registerHooks } from "node:module";

const schema = new URL("../schema.ts", import.meta.url).href;

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@candid-core/schema") {
      return { url: schema, shortCircuit: true };
    }
    return nextResolve(specifier, context);
  },
});
