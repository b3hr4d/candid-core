# @candid-core/cli

Candid `.did` interfaces to [`@candid-core/schema`] runtimes, with no Rust
toolchain: the [candid-core] compiler and its TypeScript generator compiled
to WebAssembly, usable as a Node CLI and as a browser library. Data in, data
out — no eval, no network, and nothing generated is ever executed.

> **0.2.0, the `latest` release.** This README describes 0.2.0. Upgrading
> from 0.1.0 is a breaking change: 0.1.0 emits modules in the old layout with
> an object-shaped principal type, refuses a whole interface with
> `ts_generation_refused` for one declaration it cannot represent, and returns
> no `omitted` list. The modules this release emits need
> `@candid-core/schema` 0.3.0 exactly, which is the optional peer it declares;
> install the two as a pair, each with `--save-exact`:
> `npm install --save-exact @candid-core/schema` and
> `npm install --save-dev --save-exact @candid-core/cli`.
>
> `@candid-core/schema` 0.3 breaks the 0.2 API (principal values are canonical text, collapsing opts are boxed, every `vec nat8` is a `Uint8Array`, generated modules use a new binding layout and may omit declarations, and the `./actor`, `./transport-icp`, `./forms` and `./labels` subpaths are gone); there is no compatibility layer.
>
> Every change since 0.1.0 is recorded under `0.2.0` and `0.2.0-beta.1` in
> the [changelog](./CHANGELOG.md), which ships in the tarball, and the
> [migration notes](https://b3hr4d.github.io/candid-core/migrating-from-0-2.html)
> show what each break means for your code, with before and after code where
> the compiler shows it.

```sh
npx @candid-core/cli gen ./service.did -o ./generated
```

`gen` takes one or more entries (`gen <a.did> [<b.did> …]`), and two flags:
`--json` prints one machine-readable document instead of the human report,
and `--check` compares instead of writing. See [The command](#the-command).

That emits three things:

- **`service.ts`** — the generated `@candid-core/schema` module: one reviewed
  type alias and one invariantly-annotated schema builder per declaration,
  both exported under its Candid name (the alias declared as that name, so
  compiler errors read it; the builder bound as a `$`-prefixed local), with
  the `.did`'s doc comments as JSDoc, and, for a service with an actor, the
  `actor` schema and the `Actor` call interface, each method of which carries
  its mode (`$.WithMode<"query">`); byte-identical to what the Rust-native
  generator emits;
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

## The command

```text
candid-core-cli gen <service.did>... [-o <dir>] [--json] [--check]
```

**Several entries.** Each entry generates its own module and envelope, named
from its file: `a/service.did` writes `<dir>/service.ts` and
`<dir>/service.envelope.json`, into the one `-o` directory (default: the
current directory). The output of an entry is byte-for-byte what a run of that
entry alone produces. Two decisions here are recommendations the maintainer
can overturn, each implemented in one place in `bin/cli.js`:

- *Bundle roots.* An entry's bundle is the `.did` files beneath the entry's
  own directory, and the bundle limits (256 files, 1 MiB per file, 8 MiB in
  all) apply to each entry's bundle separately. Entries in the same directory
  share one read of it, so they are not counted twice against each other.
  Inside a file, a run of more than 256 consecutive comments between two
  tokens is refused with `resource_limit_exceeded` (resource
  `source_nesting`), since 0.2.0; write a long header as one `/* */` block,
  which counts as one comment.
- *Duplicate stems.* Two entries with the same stem (`a/service.did` and
  `b/service.did`, or the same file twice) would write the same output, so the
  run is refused as a usage error (exit 64) before any work. Stems compare
  case-insensitively, so the output also checks out cleanly on a
  case-folding filesystem. Give the entries distinct file names, or run them
  into separate `-o` directories.

**Files are written only when they change.** An output that already holds
exactly the generated bytes is left alone (its modification time included),
and the human report prints `unchanged <path>` instead of `wrote <path>`.

**Exit codes.** `0` when every entry generated (a module that omits
declarations still counts, and lists them), and no `--check` drift was found;
`1` when any entry failed or any checked file drifted; `64` for a usage error,
with the usage on stderr and nothing on stdout. A failing entry does not stop
the others: every entry is attempted, and the exit code is `1` if any failed.
Without `--json`, a failed entry prints its `{ ok: false, diagnostics }`
document on stdout, exactly as a single-entry run always has (several failed
entries print several documents; use `--json` for one machine-readable
result).

**Nothing is read from stdin, and nothing is written outside `-o`.** The CLI
never prompts and never waits on input, so it is safe to run as a child process
with any stdin.

### `--check`

Generate in memory and compare byte-for-byte with the files on disk. Nothing
is written (not even the output directory), and the run exits `1` if any
output file is missing or differs, listing each on stderr (`missing <path>` or
`drifted <path>`):

```sh
npx @candid-core/cli gen ./service.did -o ./generated --check
```

Use it in CI to fail when committed output is stale. A compile error is a
failed entry, not drift. Stale files for entries you no longer list are not
looked for.

### `--json`

Prints exactly one JSON document on stdout and nothing else; stderr stays
empty. The identities the human report prints are not part of it. Combine it
with `--check`. A usage error (exit 64) prints no document.

```json
{
  "schemaVersion": 1,
  "ok": true,
  "check": false,
  "entries": [
    {
      "entry": "a/service.did",
      "status": "written",
      "module": "generated/service.ts",
      "envelope": "generated/service.envelope.json",
      "omitted": [],
      "diagnostics": []
    }
  ],
  "drift": []
}
```

- `schemaVersion` is `1`. It changes only when an existing field's meaning or
  shape changes; adding a field does not change it. Refuse a version you do
  not know.
- `ok` is true exactly when no entry is `failed` or `drifted`, and the exit
  code is `0` exactly when `ok` is true.
- `check` records whether `--check` was given.
- `entries` has one object per entry, in command-line order, with `entry` (as
  given on the command line), `module` and `envelope` (the paths it writes or
  compares: `-o` joined with `<stem>.ts` and `<stem>.envelope.json`, present
  for a failed entry too), and:
  - `status`: `written` (a file was created or changed), `unchanged` (both
    files already held these bytes), `drifted` (`--check` only: a file is
    missing or differs; nothing was written) or `failed`;
  - `omitted`: what the module leaves out, exactly as `didToModule` returns it
    (see [Omissions](#omissions)), `[]` for a failed entry. Omissions never make
    an entry fail;
  - `diagnostics`: for a failed entry, the compiler's diagnostics unchanged in
    shape (see [Failures](#failures)); `[]` otherwise. A failure found by the
    CLI itself uses the same item shape with one of the codes
    `did_file_read_error` (the entry's directory cannot be read),
    `did_source_not_found` (the entry is not a `.did` file there),
    `resource_limit_exceeded` (the bundle bounds), `nondeterministic_output`
    (two generations disagreed) or `internal_error` (anything unexpected, such as
    an output that cannot be written; it carries no `phase`).
- `drift` lists every path `--check` found missing or different, in entry
  order, module before envelope; always `[]` without `--check`.

The same types ship as `CliReport` and `CliEntryReport` in the package's
declarations. A run is deterministic: the same inputs give the same files and
the same document, byte for byte, from any working directory. There are no
timestamps, and every path in the document and in the human report (including
the messages of failures such as an unreadable or missing entry) is spelled as
you passed it, or `-o` joined with a file name, never resolved to an absolute
host path: an absolute path appears only if you passed one.

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

## Projection and compatibility

> **Not in 0.2.0.** `project`, `check`, `projectDid` and `checkCompatible` are
> unreleased: they ship in the next release, recorded under `Unreleased` in
> the [changelog](./CHANGELOG.md).

Two commands for an interface you do not own, such as a canister's published
`.did`. Both work on files already on disk: the tool never fetches anything,
so getting the live interface is your build step's job.

```text
candid-core-cli project <in.did> --methods <a,b,...> -o <out.did> [--json]
candid-core-cli check <written.did> --against <live.did> [--json]
```

**`project`** writes a `.did` holding only the methods you name and every
declaration they reach, as one self-contained file (imports are inlined), and
prints the interface identities of the input and of the projection. Feed the
result to `gen` like any `.did`: the generated `Actor` lists only those
methods, each with its mode. The output is deterministic: the same input and
the same set of names give the same bytes, whatever order you name them in.
Declaration names, doc comments and argument names are kept; a service
class's init arguments are not, since a client never sends them. Methods are
written in name order. An unknown
method name, or an empty list (`--methods ""`), fails with exit 1 and a
diagnostic (`unknown_method`, whose `notes` list the service's methods, or
`empty_method_list`), and writes nothing. `-o` is required, and may not be,
or become, a source of the input: `project` reads the input as `gen` does (the
file and every `.did` beneath its directory), so an `-o` that names one of
those files by any path, symlink or hard link, or a new `.did` anywhere beneath
that directory, fails with exit 1 and `output_is_input` (`path` is the `-o`
given, `notes` the existing sources it names, empty for a new file) and writes
nothing. Write the projection outside the input's directory. An
output that cannot be read or written (a directory, a read-only parent, a full
disk) fails with exit 1 and `output_write_failed`, whose `notes` hold the
system error code, such as `EISDIR` or `EACCES`; without `--json` it is one
`cannot write …` line on stderr, as a source that cannot be read is
`cannot read …`. The file is written only when it changes; `--json` prints
`{ ok, output, status, methods, input, projection }` instead of the report,
and `{ ok: false, diagnostics }` for any failure.

```sh
npx @candid-core/cli project ./live/ledger.did --methods icrc1_balance_of,icrc1_transfer -o ./src/ledger.did
```

**`check`** exits `0` when the live interface is still a Candid subtype of the
written one, and `1` otherwise. Every method of the written `.did` must exist
in the live one with the same mode, its arguments contravariant and its
results covariant; methods only the live service has are ignored, and so is
a class's init. So an added method or an added `opt` result field passes,
and a removed method, a changed mode, or a result that gained a variant arm
fails. A projection is not the only valid written side: a hand-written
subset `.did` checks the same way. The report prints both interface
identities on stdout, one line per finding on stderr, and a verdict line:

```text
written: candid-core:interface:v1:sha256:…
live:    candid-core:interface:v1:sha256:…
error: icrc1_transfer: method_incompatible at $results[0].Err.TooOld: the other variant has no such arm
incompatible: 1 error(s), 0 warning(s)
```

Each finding names its method and a stable code: `method_missing`,
`mode_changed` and `method_incompatible` are errors; `special_opt_rule` is a
warning, for a change Candid accepts only by reading the value under an `opt`
as `null` (such as `opt nat` becoming `opt text`), so it passes but loses the
data at that path, and it is reported at every such path along which no pair
of types repeats (a changed alias under each field that uses it; a recursive
type up to where a path comes back round); `resource_limit_exceeded` fails a
method whose check reached a bound, its depth or its work. Past 1,000
warnings a method reports the first 1,000 and one `resource_limit_exceeded`
warning (`check_warnings`) saying the rest were dropped; its verdict stands.
`method_incompatible` and `special_opt_rule` carry the path
into the method's type where the check failed: `$args[i]` or `$results[i]`,
then `.name` per record field or variant arm (`["name"]` when it is not an
identifier, `[id]` for a numeric label), `[*]` per `vec` element, `?` per
`opt` content, and `::args[i]`, `::results[i]` or `::name` into a `func` or
`service` type. `--json` prints the `checkCompatible` document below.

The live interface identity tells an unchanged interface from one that
changed compatibly: `project` prints the input's, so record it when you write
the projection, and compare it with what `check` prints later.

```js
import { checkCompatible, projectDid } from "@candid-core/cli";

const projected = await projectDid(liveDid, ["icrc1_balance_of", "icrc1_transfer"]);
// → { ok: true, did, methods, input, projection } — the text, the methods it
//   holds, and { contract_id, interface_id } of the input and the projection;
//   or { ok: false, diagnostics }.

const report = await checkCompatible(writtenDid, liveDid);
// → { ok: true, compatible, written, live, diagnostics }, each diagnostic
//   { code, severity, method, path?, message }; or { ok: false, input,
//   diagnostics } when one side does not compile (`input` names it).
```

Both take Candid text or `{ entry, files }`, as `didToModule` does. The check
is one Rust implementation compiled into this package's WebAssembly, tested
against the subtype check of the `candid` crate it pins and against the one
`@candid-core/schema`'s decoder runs on a `func` or `service` reference. Where the `candid` crate accepts a pair that is not a
subtype (its coinductive memo can keep a pair proven under an assumption it
later retracted), this check refuses it, as the decoder does.

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
omission, not a failure. The CLI exits 1 when any entry failed, printing each
failing document on stdout (or all of them in the `--json` document);
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

The `@candid-core/schema` peer is an exact version, not a range, while the two
packages move in lockstep: each schema release is paired with a new release
of this package that names it, so a generated module always meets the runtime
it was generated for.

[candid-core]: https://github.com/b3hr4d/candid-core
[`@candid-core/schema`]: https://www.npmjs.com/package/@candid-core/schema
