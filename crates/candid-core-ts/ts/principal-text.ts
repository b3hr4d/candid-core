// The canonical textual form of a Candid principal, implemented once.
// Internal: the root entry (`principal`, `isPrincipal`), the validator and
// the codec import it, so it ships inside `dist/`, but it is not a package
// export — the codec re-exports its two functions under their original names.
//
// The form is fixed by the Internet Computer interface specification: the
// big-endian CRC-32 of the id bytes, followed by the id bytes, encoded as
// lowercase RFC 4648 base32 without padding, split into groups of five
// characters joined by `-`. An id is 0 to 29 bytes long, so canonical text is
// 8 to 63 characters: `aaaaa-aa` is the management canister (no bytes) and
// `2vxsx-fae` the anonymous principal (the single byte 0x04).
//
// Self-contained by design (recorded on issue #103): no runtime dependency on
// any Principal implementation. Parsing is strict rather than forgiving: text
// is canonical exactly when it parses and re-renders to itself, so upper
// case, a missing or misplaced dash, a checksum mismatch, non-zero padding
// bits in the last character, and an over-long id are all refused rather than
// normalized.

const BASE32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

/** The longest canonical text: a 29-byte id is 53 base32 characters plus 10 dashes. */
const MAX_TEXT_LENGTH = 63;

const CRC32_TABLE: Uint32Array = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc = CRC32_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/**
 * Canonical principal text for raw id bytes: lowercase base32 of the CRC-32
 * checksum followed by the bytes, dash-grouped by five. An id longer than 29
 * bytes renders too, but that text is not a principal and every parser in
 * this package refuses it.
 */
export function principalTextFromBytes(bytes: Uint8Array): string {
  const checksum = crc32(bytes);
  const data = new Uint8Array(4 + bytes.length);
  data[0] = (checksum >>> 24) & 0xff;
  data[1] = (checksum >>> 16) & 0xff;
  data[2] = (checksum >>> 8) & 0xff;
  data[3] = checksum & 0xff;
  data.set(bytes, 4);
  let encoded = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of data) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      encoded += BASE32_ALPHABET[(buffer >> bits) & 31];
    }
  }
  if (bits > 0) {
    encoded += BASE32_ALPHABET[(buffer << (5 - bits)) & 31];
  }
  let grouped = "";
  for (let i = 0; i < encoded.length; i += 5) {
    grouped += (i > 0 ? "-" : "") + encoded.slice(i, i + 5);
  }
  return grouped;
}

/**
 * Raw id bytes of canonical principal text, or `undefined` when the text is
 * not canonical: wrong alphabet or grouping, checksum mismatch, an id longer
 * than the 29-byte maximum, or any re-rendering difference (case included).
 * Callers fail closed on `undefined` rather than guessing.
 */
export function principalBytesFromText(text: string): Uint8Array | undefined {
  // Every canonical text is at most 63 characters, so a longer string is
  // refused before any work proportional to its length.
  if (text.length > MAX_TEXT_LENGTH) {
    return undefined;
  }
  const compact = text.split("-").join("");
  let buffer = 0;
  let bits = 0;
  const data: number[] = [];
  for (const char of compact) {
    const index = BASE32_ALPHABET.indexOf(char);
    if (index < 0) {
      return undefined;
    }
    buffer = (buffer << 5) | index;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      data.push((buffer >> bits) & 0xff);
    }
  }
  if (data.length < 4 || data.length > 33) {
    return undefined;
  }
  const bytes = Uint8Array.from(data.slice(4));
  return principalTextFromBytes(bytes) === text ? bytes : undefined;
}
