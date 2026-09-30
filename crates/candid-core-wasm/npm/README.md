# @candid-core/cli

Candid `.did` interfaces to [`@candid-core/schema`] runtimes, with no Rust
toolchain: the [candid-core] compiler and its TypeScript generator compiled
to WebAssembly, usable as a Node CLI and as a browser library. Data in, data
out — no eval, no network, and nothing generated is ever executed.

```sh
npx @candid-core/cli gen ./service.did -o ./generated
```

That emits three things:

- **`service.ts`** — the generated `@candid-core/schema` module: one reviewed
  type alias and one invariantly-annotated schema builder per declaration,
  byte-identical to what the Rust-native generator emits;
- **`service.envelope.json`** — the one-document `ContractEnvelope`: the
  canonical Contract plus its field-name table under the
  `org.candid-core.field-names/v1` extension, byte-identical to
  `candid-core compile ./service.did --envelope`, ready to hand whole to
  `schemaFromContract`;
- the printed **content-addressed identities** (`contract`, and `interface`
  when the service has an actor) — the same
  `candid-core:contract:v1:sha256:…` addresses the Rust toolchain computes,
  usable to pin an interface and detect drift.

A declaration the generator cannot represent does not cost the whole
module: it is left out, together with every declaration and actor method that
references it, and the run still succeeds (exit 0). The CLI prints one
`warning: omitted …` line per entry on stderr, and the module's header lists
the same entries as `// Omitted:` lines — for example
`// Omitted: type Holder (references_omitted via Bad)`. See
[Omissions](#omissions).

Imports are resolved from the entry file's directory: every `.did` beneath
it is handed to the compiler as an in-memory bundle, so relative imports
work without any filesystem access from the wasm side. Every generation runs
twice and the tool refuses to write on any byte mismatch — determinism is
enforced, not assumed.

## The library

```js
import { didToContract, didToModule } from "@candid-core/cli";

const envelope = await didToContract("service : { ping : () -> () };");
// → the ContractEnvelope document ("contract" in envelope), or
//   { ok: false, diagnostics } with the compiler's diagnostics verbatim.

const generated = await didToModule({
  entry: "main.did",
  files: {
    "main.did": 'import "types.did";\nservice : { get : () -> (Item) query };',
    "types.did": "type Item = record { id : nat };",
  },
});
// → { ok: true, module, omitted } with the generated TypeScript text and
//   what it left out (empty here), or { ok: false, diagnostics }.
```

Both functions accept either a string of Candid text or
`{ entry, files: { name: text } }` for a multi-file bundle. In a browser the
wasm is fetched relative to the module automatically; call
`init(bytesOrUrl)` first to supply it yourself. Feed `didToContract`'s
result straight to `schemaFromContract` from [`@candid-core/schema`] — the
envelope carries the field names, so one document is the whole hand-off:

```js
import { didToContract } from "@candid-core/cli";
import { schemaFromContract } from "@candid-core/schema/contract";

const built = schemaFromContract(await didToContract(didText));
```

## Omissions

`didToModule`'s success carries `omitted`, an array of
`{ kind, name, reason, via? }` — declarations first, then methods, each
sorted by name, and empty when nothing is left out:

- `kind` is `"declaration"` or `"method"` (a method of the actor's service,
  dropped from both the `actor` schema and the `Actor` type);
- `reason` is one of a closed set: `reserved_field_name` (a record field or
  variant arm named like the `_N_` numeric-id rendering, which as a schema
  key would read back as the wrong wire id), `ambiguous_variant_arm` (an arm
  whose payload is a declared `opt` of an uninhabited type, whose generated
  type could not tell it from a bare tag), `reserved_export_name` (a
  declaration named `actor` or `Actor`, the module's own export names), or
  `references_omitted` (the entry references another omitted declaration);
  `invalid_declaration_name` completes the set for Contract documents, which
  Candid source never produces;
- `via`, present only for `references_omitted`, names the omitted declaration
  referenced.

Omission follows every reference, nested `func` and `service` types
included, up to the containing declaration: a record holding a
`service { f : (Bad) -> () }` is omitted whole rather than losing `f`,
because a service value's wire type is its full method table. Everything the
module still emits is exactly what it would be had the omitted declarations
never existed. `schemaFromContract` leaves out the same entries for the same
contract and lists them the same way.

## Failures

Nothing throws for data errors and nothing half-succeeds: a compile failure
is `{ ok: false, diagnostics }` with [candid-core]'s structured diagnostics
passed through verbatim (stable codes such as `did_parse_error`,
`did_file_read_error` analogues, resource bounds included), a malformed
request is `invalid_request`, and `ts_generation_refused` is reserved for a
generator refusal of an invalid contract graph — which a contract compiled
from Candid source never is, so an unrepresentable declaration is an
omission, not a failure. The CLI prints the failing document on stdout and exits 1;
usage errors exit 64 with usage on stderr, in the native binary's
convention.

## Version pairing

This package embeds exact revisions, recorded per release in
[CHANGELOG.md](./CHANGELOG.md): the `candid-core` compiler crate and the
`candid-core-ts` generator (unpublishable on crates.io by design; embedding
it in this wasm artifact is an owner decision recorded on the repository's
issue tracker) are built from one repository commit, and the parity gates
prove the emitted module and contract are byte-identical to that commit's
Rust-native outputs over the golden fixtures.

[candid-core]: https://github.com/b3hr4d/candid-core
[`@candid-core/schema`]: https://www.npmjs.com/package/@candid-core/schema
