// Option checking for every public entry point that takes an options object
// (issue #190). Internal: `validate`, the codec and the Contract loader import
// it, so it ships inside `dist/`, but it is not a package export.
//
// Options are code, not input. A misspelled limit that is silently ignored
// applies a policy nobody asked for, and a `NaN` limit switches its bound off
// entirely (every `observed > NaN` is false). Both are programmer errors, so
// both throw `TypeError` before the entry point reads anything else — the
// convention `resolveSchema`, `serviceMethods` and `unwrapResult` already
// follow — rather than growing the closed issue-code unions for something no
// untrusted value or byte string can cause. This is the TypeScript side of
// ADR 0005's rule that unknown configuration fields are rejected, never
// ignored.
//
// A limit is a non-negative safe integer. `0` is a defined fail-closed policy
// (it refuses any input that consumes the resource), exactly as in the Rust
// `Limits`. A property whose value is `undefined` is treated as absent, so
// `{ maxDepth: setting }` with an unset setting still means the default.
//
// Every known key is read exactly once, into the frozen snapshot
// `checkOptions` returns; entry points pass that snapshot down and never read
// the caller's object again (a check-then-reread would let a getter or Proxy
// pass the check with `0` and run with `NaN`). A caller's object that throws
// while being read — a getter, a Proxy trap, a revoked Proxy — is a
// programmer error like the rest, and surfaces as a `TypeError` naming the
// entry point with the thrown value as its `cause`, from every entry point.

/**
 * Whether `value` is an acceptable limit. The one place the `Infinity`
 * question (issue #190, sub-decision D3) is answered: the recommendation
 * applied here is *no* — a limit is a safe integer, and a trusted host that
 * wants no practical bound passes a large one. Admitting `Infinity` is this
 * one line: `Number.isSafeInteger(value) || value === Infinity`.
 */
function isLimitValue(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/** A short description of a rejected option value, never its contents. */
function describeOption(value: unknown): string {
  if (typeof value === "number") {
    return String(value);
  }
  if (typeof value === "string") {
    return `the string ${JSON.stringify(value)}`;
  }
  if (value === null) {
    return "null";
  }
  return `a ${typeof value}`;
}

/**
 * Check an entry point's options object up front and return a *snapshot* of
 * it: a frozen, null-prototype record holding exactly the known keys, each
 * read from the caller's object once. The entry point, and everything it
 * calls, reads only the snapshot — never the caller's object again — so a
 * getter or Proxy that answers differently on a second read cannot slip an
 * unchecked value (a `NaN` limit, say) past the check.
 *
 * Throws `TypeError` when `options` is not an object, when it has an own key
 * outside `limits` and `other`, or when a `limits` key holds anything but
 * `undefined` or a non-negative safe integer. Keys in `other` are copied
 * unchecked; their values are the entry point's to check (the Contract
 * loader's `names` table is data, validated into issues).
 *
 * Reading the object is itself guarded: a getter or Proxy trap that throws
 * while the options are read (`ownKeys`, `get`, or the array check) surfaces
 * as a `TypeError` naming the entry point, with the original exception as its
 * `cause` — the same error type every other options failure raises, from
 * every entry point.
 */
export function checkOptions(
  entry: string,
  options: unknown,
  limits: readonly string[],
  other: readonly string[] = [],
): { readonly [key: string]: unknown } {
  if (typeof options !== "object" || options === null) {
    throw new TypeError(`${entry}: options must be an object, got ${describeOption(options)}`);
  }
  const record = options as { readonly [key: string]: unknown };
  let isArray: boolean;
  let ownKeys: readonly (string | symbol)[];
  const snapshot: { [key: string]: unknown } = Object.create(null);
  try {
    isArray = Array.isArray(record);
    ownKeys = isArray ? [] : Reflect.ownKeys(record);
  } catch (error) {
    throw unreadable(entry, error);
  }
  if (isArray) {
    throw new TypeError(`${entry}: options must be an object, got an array`);
  }
  for (const key of ownKeys) {
    if (typeof key !== "string" || (!limits.includes(key) && !other.includes(key))) {
      throw new TypeError(
        `${entry}: unknown option ${typeof key === "string" ? JSON.stringify(key) : String(key)}` +
          ` (known options: ${[...limits, ...other].join(", ")})`,
      );
    }
  }
  // The one read of each known key. Inherited values are honoured, as a plain
  // property read always did; what is read here is what the call uses.
  try {
    for (const key of [...limits, ...other]) {
      snapshot[key] = record[key];
    }
  } catch (error) {
    throw unreadable(entry, error);
  }
  for (const key of limits) {
    const value = snapshot[key];
    if (value !== undefined && !isLimitValue(value)) {
      throw new TypeError(
        `${entry}: option ${key} must be a non-negative safe integer, got ${describeOption(value)}`,
      );
    }
  }
  return Object.freeze(snapshot);
}

/** The `TypeError` for an options object that threw while being read. */
function unreadable(entry: string, error: unknown): TypeError {
  const wrapped = new TypeError(`${entry}: the options object threw while being read`);
  Object.defineProperty(wrapped, "cause", {
    value: error,
    writable: true,
    enumerable: false,
    configurable: true,
  });
  return wrapped;
}
