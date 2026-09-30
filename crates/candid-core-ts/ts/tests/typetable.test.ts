// The structural type table (issue #190), as a unit: `canonicalTypeTable`
// merges repeated structure, keeps cycles finite, never recurses on the host
// stack, and does work linear in its input. Work is counted, never timed
// (no timing measurement gates anything here, recorded on #39); the
// end-to-end byte properties live in codec.test.ts, section 5.

import { test } from "node:test";
import assert from "node:assert/strict";

import { canonicalTypeTable, type TableEntry } from "../typetable.ts";

const OPT = 0x6e;
const VEC = 0x6d;
const RECORD = 0x6c;
const NAT = 0x7d;
const NAT8 = 0x7b;

/** `opt <ref>` as a provisional entry. */
function opt(ref: number): TableEntry {
  return { segments: [[OPT], []], refs: [ref] };
}

/** `vec <ref>` as a provisional entry. */
function vec(ref: number): TableEntry {
  return { segments: [[VEC], []], refs: [ref] };
}

/** `vec nat8`, a leaf. */
const BLOB: TableEntry = { segments: [[VEC, NAT8]], refs: [] };

/** Unsigned LEB128, for field counts and ids in hand-built entries. */
function leb(value: number): number[] {
  const out: number[] = [];
  let v = value;
  do {
    let byte = v % 128;
    v = Math.floor(v / 128);
    if (v > 0) {
      byte |= 0x80;
    }
    out.push(byte);
  } while (v > 0);
  return out;
}

/** `record { i : <refs[i]> }` with field ids 0..n-1. */
function record(refs: readonly number[]): TableEntry {
  const segments: number[][] = [[RECORD, ...leb(refs.length), ...leb(0)]];
  for (let i = 1; i < refs.length; i += 1) {
    segments.push(leb(i));
  }
  segments.push([]);
  return { segments, refs: [...refs] };
}

/** The input's own size: one per entry, reference, and literal byte. */
function sizeOf(entries: readonly TableEntry[]): number {
  let size = 0;
  for (const entry of entries) {
    size += 1 + entry.refs.length;
    for (const segment of entry.segments) {
      size += segment.length;
    }
  }
  return size;
}

test("repeated structure merges; the first-visit order of what remains is kept", () => {
  // record { 0: opt blob; 1: opt blob } with each `opt blob` its own entry:
  // five provisional entries, three distinct types.
  const entries = [record([1, 3]), opt(2), BLOB, opt(4), BLOB];
  const { table, roots } = canonicalTypeTable(entries, [0]);
  assert.deepStrictEqual(roots, [0]);
  assert.deepStrictEqual(table, [
    [RECORD, 2, 0, 1, 1, 1],
    [OPT, 2],
    [VEC, NAT8],
  ]);
});

test("primitive roots pass through, and repeated roots map to one entry", () => {
  const entries = [opt(1), BLOB, opt(3), BLOB];
  const { table, roots } = canonicalTypeTable(entries, [-3, 0, 2, -3]);
  assert.deepStrictEqual(roots, [-3, 0, 0, -3]);
  assert.strictEqual(table.length, 2);
});

test("an unrolled copy leading into a cycle merges into the cycle", () => {
  // 0: opt -> 1, 1: record { 0: nat; 1: -> 2 }, 2: opt -> 3,
  // 3: record { 0: nat; 1: -> 2 } — a list entered through one unrolling.
  const list = (next: number): TableEntry => ({
    segments: [[RECORD, 2, 0, NAT, 1], []],
    refs: [next],
  });
  const entries = [opt(1), list(2), opt(3), list(2)];
  const { table, roots } = canonicalTypeTable(entries, [0]);
  assert.deepStrictEqual(roots, [0]);
  assert.deepStrictEqual(table, [
    [OPT, 1],
    [RECORD, 2, 0, NAT, 1, 0],
  ]);
});

test("a deep chain is canonicalized without host recursion", () => {
  // 200_000 nested vecs: far past any host stack if either pass recursed.
  const depth = 200_000;
  const entries: TableEntry[] = [];
  for (let i = 0; i < depth - 1; i += 1) {
    entries.push(vec(i + 1));
  }
  entries.push(BLOB);
  const { table, work } = canonicalTypeTable(entries, [0]);
  assert.strictEqual(table.length, depth);
  assert(work <= 4 * sizeOf(entries), `work ${work} for input size ${sizeOf(entries)}`);
});

test("a long cycle is canonicalized without host recursion", () => {
  const length = 200_000;
  const entries: TableEntry[] = [];
  for (let i = 0; i < length; i += 1) {
    entries.push(vec((i + 1) % length));
  }
  const { table } = canonicalTypeTable(entries, [0]);
  // Every entry is `vec` of the next; only the anchor closes the loop, so
  // this pass keeps the whole cycle (minimising it is a non-goal).
  assert.strictEqual(table.length, length);
  assert.deepStrictEqual(table[length - 1], [VEC, 0]);
});

test("work is linear in the input: wide, repetitive, and deep tables", () => {
  const cases: [string, (n: number) => TableEntry[]][] = [
    [
      // One record over n fresh copies of `opt blob`: 2n + 1 entries, 3 kept.
      "wide and repetitive",
      (n) => {
        const entries: TableEntry[] = [record(Array.from({ length: n }, (_, i) => 1 + 2 * i))];
        for (let i = 0; i < n; i += 1) {
          entries.push(opt(entries.length + 1), BLOB);
        }
        return entries;
      },
    ],
    [
      // n distinct records, each over the previous: nothing merges.
      "distinct",
      (n) =>
        Array.from({ length: n }, (_, i) =>
          i === n - 1 ? record([]) : { ...record([i + 1]), segments: [[RECORD, 1, ...leb(i)], []] },
        ),
    ],
  ];
  for (const [name, build] of cases) {
    for (const n of [1_000, 10_000, 100_000]) {
      const entries = build(n);
      const { work } = canonicalTypeTable(entries, [0]);
      const size = sizeOf(entries);
      assert(work <= 4 * size, `${name} at ${n}: work ${work} exceeds 4x input size ${size}`);
    }
  }
});
