// The single link point to the schema core recorded on issue #38. The core
// lives inside the generator crate's type-check harness until the npm package
// and its permanent name exist (issue #106); this re-export is the only file
// in the prototype that spells that path, so extraction later is a one-line
// change here and a specifier change in generated modules.
export { c } from "../../../crates/candid-core-ts/ts/schema.ts";
export type {
  Schema,
  Infer,
} from "../../../crates/candid-core-ts/ts/schema.ts";
