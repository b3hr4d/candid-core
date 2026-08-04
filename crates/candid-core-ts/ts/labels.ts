// Candid field labels, as this runtime renders them and the wire needs them.
//
// The semantic Contract stores only numeric label ids; schema objects store
// only rendered keys. The two are bridged by exactly two rules, both fixed by
// the ecosystem and recorded on issue #103:
//
// - a key `_N_` (underscore, decimal digits, underscore) renders the numeric
//   label id N — the convention for fields with no source name;
// - any other key is a source name, whose id is the Candid label hash
//   `h(name) = fold(h * 223 + byte) mod 2^32` over the name's UTF-8 bytes.
//
// A *source* name that is itself `_N_`-shaped hashes as a name in Candid but
// becomes indistinguishable from the id rendering once erased to a key, so
// the loader refuses such names outright (`invalid_name_table`) — the
// ambiguity would otherwise let the codec silently encode a wrong wire id.

/** The Candid label hash of a field name's UTF-8 bytes. */
export function candidLabelHash(name: string): number {
  const bytes = utf8Bytes(name);
  let hash = 0;
  for (const byte of bytes) {
    // hash < 2^32, so hash * 223 + byte < 2^40 stays float64-exact; the
    // unsigned shift is the mod-2^32 reduction.
    hash = (hash * 223 + byte) >>> 0;
  }
  return hash;
}

/**
 * The numeric label id a `_N_`-shaped key renders, or `undefined` when the
 * key is not id-shaped. `N` must be a canonical decimal below 2^32 — a
 * leading zero or an out-of-range number is not a rendering this runtime
 * ever produces, so such keys fall through to name hashing.
 */
export function numericKeyId(key: string): number | undefined {
  const match = /^_([0-9]+)_$/.exec(key);
  if (match === null) {
    return undefined;
  }
  const digits = match[1];
  if (digits.length > 1 && digits.startsWith("0")) {
    return undefined;
  }
  const id = Number(digits);
  return Number.isSafeInteger(id) && id < 4_294_967_296 ? id : undefined;
}

/** Whether a *source name* is `_N_`-shaped and therefore refused as a key. */
export function isNumericShapedName(name: string): boolean {
  return numericKeyId(name) !== undefined;
}

/**
 * The wire label id for a schema key: `_N_` parse-back first, else the name
 * hash. Total — every key derives some id; collisions are the caller's to
 * reject.
 */
export function fieldIdOfKey(key: string): number {
  return numericKeyId(key) ?? candidLabelHash(key);
}

/**
 * UTF-8 bytes of a string, refusing lone surrogates: Candid text is a
 * sequence of Unicode scalar values, so an unpaired surrogate has no
 * encoding. Returns `undefined` for such strings — the caller fails closed.
 */
export function utf8BytesStrict(text: string): Uint8Array | undefined {
  for (let i = 0; i < text.length; i += 1) {
    const unit = text.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = i + 1 < text.length ? text.charCodeAt(i + 1) : 0;
      if (next < 0xdc00 || next > 0xdfff) {
        return undefined;
      }
      i += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return undefined;
    }
  }
  return utf8Bytes(text);
}

// Hand-rolled UTF-8: the harness deliberately has no dependency surface
// beyond ECMAScript builtins and the hand-declared node stubs, and rolling
// the transform keeps byte-level behavior (surrogate policy above, exact
// lengths) in reviewable code rather than behind a host object.
function utf8Bytes(text: string): Uint8Array {
  const out: number[] = [];
  for (const char of text) {
    const point = char.codePointAt(0) as number;
    if (point < 0x80) {
      out.push(point);
    } else if (point < 0x800) {
      out.push(0xc0 | (point >> 6), 0x80 | (point & 0x3f));
    } else if (point < 0x10000) {
      out.push(0xe0 | (point >> 12), 0x80 | ((point >> 6) & 0x3f), 0x80 | (point & 0x3f));
    } else {
      out.push(
        0xf0 | (point >> 18),
        0x80 | ((point >> 12) & 0x3f),
        0x80 | ((point >> 6) & 0x3f),
        0x80 | (point & 0x3f),
      );
    }
  }
  return Uint8Array.from(out);
}

/**
 * Decode UTF-8 strictly: reject overlong forms, surrogate code points,
 * values above U+10FFFF, and truncated sequences. Returns `undefined` on any
 * violation — wire text that is not well-formed fails closed.
 */
export function utf8Decode(bytes: Uint8Array): string | undefined {
  let text = "";
  let i = 0;
  while (i < bytes.length) {
    const lead = bytes[i];
    let point: number;
    let extra: number;
    let min: number;
    if (lead < 0x80) {
      point = lead;
      extra = 0;
      min = 0;
    } else if (lead >= 0xc2 && lead <= 0xdf) {
      point = lead & 0x1f;
      extra = 1;
      min = 0x80;
    } else if (lead >= 0xe0 && lead <= 0xef) {
      point = lead & 0x0f;
      extra = 2;
      min = 0x800;
    } else if (lead >= 0xf0 && lead <= 0xf4) {
      point = lead & 0x07;
      extra = 3;
      min = 0x10000;
    } else {
      return undefined;
    }
    for (let k = 1; k <= extra; k += 1) {
      const cont = bytes[i + k];
      if (cont === undefined || (cont & 0xc0) !== 0x80) {
        return undefined;
      }
      point = (point << 6) | (cont & 0x3f);
    }
    if (point < min || point > 0x10ffff || (point >= 0xd800 && point <= 0xdfff)) {
      return undefined;
    }
    text += String.fromCodePoint(point);
    i += 1 + extra;
  }
  return text;
}
