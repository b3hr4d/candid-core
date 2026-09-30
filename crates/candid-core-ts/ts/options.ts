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
 * Check an entry point's options object up front and return it as a plain
 * record to read limits from. Throws `TypeError` when `options` is not an
 * object, when it has an own key outside `limits` and `other`, or when a
 * `limits` key holds anything but `undefined` or a non-negative safe integer.
 * Keys in `other` are allowed but their values are the entry point's to check
 * (the Contract loader's `names` table is data, validated into issues).
 */
export function checkOptions(
  entry: string,
  options: unknown,
  limits: readonly string[],
  other: readonly string[] = [],
): { readonly [key: string]: unknown } {
  if (typeof options !== "object" || options === null || Array.isArray(options)) {
    throw new TypeError(`${entry}: options must be an object, got ${describeOption(options)}`);
  }
  const record = options as { readonly [key: string]: unknown };
  for (const key of Reflect.ownKeys(record)) {
    if (typeof key !== "string" || (!limits.includes(key) && !other.includes(key))) {
      throw new TypeError(
        `${entry}: unknown option ${typeof key === "string" ? JSON.stringify(key) : String(key)}` +
          ` (known options: ${[...limits, ...other].join(", ")})`,
      );
    }
  }
  for (const key of limits) {
    const value = record[key];
    if (value !== undefined && !isLimitValue(value)) {
      throw new TypeError(
        `${entry}: option ${key} must be a non-negative safe integer, got ${describeOption(value)}`,
      );
    }
  }
  return record;
}
