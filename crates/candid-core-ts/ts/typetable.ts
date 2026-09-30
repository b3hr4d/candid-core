// The encoder's structural type table (issue #190). Internal: the codec
// imports it, so it ships inside `dist/`, but it is not a package export.
//
// # Why
//
// The encoder walks the schema graph once, keyed by object identity, and
// builds a *provisional* table: one entry per distinct composite schema node
// it meets, in first-visit (preorder) order. Identity is the wrong key for the
// bytes. A generated module mints a fresh `c.opt(c.blob())` for every field
// that spells one, while `schemaFromContract` shares one node per Contract
// arena entry, so the same value encoded through the two gave two different
// type tables (the ledger fixture's `TransferArg`: 11 entries against 7).
// Anything that keys a cache on argument bytes saw two keys for one call.
//
// # The canonical form
//
// `canonicalTypeTable` rewrites the provisional table into one that depends
// only on the Candid types, in two linear passes:
//
// 1. Hash-consing, post-order. An entry's key is its literal bytes with each
//    child reference replaced by the child's canonical id, so two entries are
//    merged exactly when they have the same opcode, the same field ids,
//    method names, mode and counts, and children that were themselves merged.
//    Entries with no cycle beneath them are therefore maximally shared: the
//    table is minimal for acyclic structure.
// 2. Numbering, pre-order. The canonical graph is walked depth-first from the
//    argument types in order, children in wire order (fields by id, methods
//    by name bytes, func arguments before results) — exactly the order the
//    provisional walk used — and each entry takes the next index when first
//    reached. A schema with no repeated structure keeps the table, byte for
//    byte, that the identity walk always produced; only repeated structure
//    collapses.
//
// # Cycles
//
// A recursive type is a cycle in the provisional table, closed where the
// walk re-entered an entry still open on its stack. That entry becomes the
// cycle's *anchor*: its canonical id is fixed when the back edge is first
// seen, so everything inside the cycle is keyed against it, and the anchor's
// own entry is registered under its key once complete. Unrolled copies
// leading into a cycle (a generated alias re-running its target's `rec` body,
// say) hash onto the cycle's entries and merge. What this pass does not do is
// minimise cyclic graphs in general: two separately built knots for one
// recursive type, or a knot whose cycle is a multiple of a shorter
// equivalent one (`type Even = record { next : opt Odd }` beside
// `type Odd = record { next : opt Even }`, built as two knots), stay
// distinct — still valid Candid, but different bytes. Bisimulation
// minimisation is the Contract canonicalizer's job, and every schema built
// from a Contract document or generated from one inherits its minimal graph,
// one knot per recursive node; issue #190 records full cyclic minimisation
// here as a non-goal.
//
// # Cost
//
// Both passes are iterative (explicit stacks, never host recursion) and
// visit each provisional entry and each reference once; hash-consing costs
// one key string per entry, as long as the entry's own bytes. Total work is
// linear in the provisional table's size, which the encoder's
// `maxTypeTableEntries` already bounds. `work` reports the count so a test
// can pin that without timing anything.

/**
 * One provisional table entry: literal wire bytes interleaved with table
 * references, `segments[0] ref[0] segments[1] … ref[n-1] segments[n]`.
 * Primitive opcodes are literal bytes; `refs` holds only table indices.
 */
export interface TableEntry {
  readonly segments: readonly (readonly number[])[];
  readonly refs: readonly number[];
}

/** The canonical table, the argument types remapped onto it, and the work spent. */
export interface CanonicalTable {
  readonly table: number[][];
  readonly roots: number[];
  /** Elementary steps taken: entries and references visited, key and output bytes. */
  readonly work: number;
}

/** Signed LEB128 of a non-negative table index. */
function writeSlebIndex(out: number[], value: number): void {
  let v = value;
  for (;;) {
    const byte = v % 128;
    v = Math.floor(v / 128);
    if (v === 0 && (byte & 0x40) === 0) {
      out.push(byte);
      return;
    }
    out.push(byte | 0x80);
  }
}

/**
 * A string of the given UTF-16 units, built in bounded chunks: spreading a
 * very wide entry into one `fromCharCode` call would overflow the engine's
 * argument list.
 */
function keyOf(units: readonly number[]): string {
  const CHUNK = 8192;
  if (units.length <= CHUNK) {
    return String.fromCharCode(...units);
  }
  let key = "";
  for (let start = 0; start < units.length; start += CHUNK) {
    key += String.fromCharCode(...units.slice(start, start + CHUNK));
  }
  return key;
}

interface CanonicalEntry {
  readonly segments: readonly (readonly number[])[];
  readonly refs: readonly number[];
}

/**
 * Rewrite a provisional type table into its structural canonical form (see
 * the module header). `roots` are the argument type references: negative
 * primitive opcodes pass through, non-negative ones index `entries`.
 */
export function canonicalTypeTable(
  entries: readonly TableEntry[],
  roots: readonly number[],
): CanonicalTable {
  const count = entries.length;
  const OPEN = 1;
  const DONE = 2;
  const state = new Uint8Array(count);
  const canonical = new Int32Array(count).fill(-1);
  const anchor = new Int32Array(count).fill(-1);
  const content: (CanonicalEntry | undefined)[] = [];
  const interned = new Map<string, number>();
  let work = 0;

  // Pass 1: hash-consing in post-order. `nodes`/`positions` are one explicit
  // stack of (entry, next child) frames.
  const nodes: number[] = [];
  const positions: number[] = [];
  for (const root of roots) {
    if (root < 0 || state[root] !== 0) {
      continue;
    }
    state[root] = OPEN;
    nodes.push(root);
    positions.push(0);
    while (nodes.length > 0) {
      const top = nodes.length - 1;
      const index = nodes[top];
      const { segments, refs } = entries[index];
      const position = positions[top];
      if (position < refs.length) {
        positions[top] = position + 1;
        work += 1;
        const child = refs[position];
        if (state[child] === 0) {
          state[child] = OPEN;
          nodes.push(child);
          positions.push(0);
        } else if (state[child] === OPEN && anchor[child] < 0) {
          // A back edge: `child` closes a cycle. Its canonical id is fixed
          // now, so the entries inside the cycle can be keyed against it.
          anchor[child] = content.length;
          content.push(undefined);
        }
        continue;
      }
      // Every child is done or is an open anchor: key this entry. The key is
      // one UTF-16 unit per literal byte (0x00–0xff) and three units per
      // canonical id, each at or above 0x8000 — fixed width and disjoint
      // from the bytes, so no two different entries share a key.
      const mapped: number[] = [];
      const units: number[] = [];
      for (let i = 0; i < segments.length; i += 1) {
        const segment = segments[i];
        for (const byte of segment) {
          units.push(byte);
        }
        work += 1 + segment.length;
        if (i < refs.length) {
          const child = refs[i];
          const id = state[child] === DONE ? canonical[child] : anchor[child];
          mapped.push(id);
          units.push(
            0x8000 | (id & 0x7fff),
            0x8000 | ((id >>> 15) & 0x7fff),
            0x8000 | Math.floor(id / 0x40000000),
          );
        }
      }
      const key = keyOf(units);
      let id: number;
      if (anchor[index] >= 0) {
        id = anchor[index];
        content[id] = { segments, refs: mapped };
        if (!interned.has(key)) {
          interned.set(key, id);
        }
      } else {
        const existing = interned.get(key);
        if (existing !== undefined) {
          id = existing;
        } else {
          id = content.length;
          content.push({ segments, refs: mapped });
          interned.set(key, id);
        }
      }
      canonical[index] = id;
      state[index] = DONE;
      nodes.pop();
      positions.pop();
    }
  }

  // Pass 2: number the canonical graph in pre-order from the roots, in the
  // same child order the provisional walk used.
  const numbered = new Int32Array(content.length).fill(-1);
  const order: number[] = [];
  const mappedRoots: number[] = [];
  for (const root of roots) {
    if (root < 0) {
      mappedRoots.push(root);
      continue;
    }
    const start = canonical[root];
    if (numbered[start] < 0) {
      numbered[start] = order.length;
      order.push(start);
      nodes.push(start);
      positions.push(0);
      while (nodes.length > 0) {
        const top = nodes.length - 1;
        const refs = (content[nodes[top]] as CanonicalEntry).refs;
        const position = positions[top];
        if (position < refs.length) {
          positions[top] = position + 1;
          work += 1;
          const child = refs[position];
          if (numbered[child] < 0) {
            numbered[child] = order.length;
            order.push(child);
            nodes.push(child);
            positions.push(0);
          }
          continue;
        }
        nodes.pop();
        positions.pop();
      }
    }
    mappedRoots.push(numbered[start]);
  }

  const table: number[][] = [];
  for (const id of order) {
    const { segments, refs } = content[id] as CanonicalEntry;
    const bytes: number[] = [];
    for (let i = 0; i < segments.length; i += 1) {
      for (const byte of segments[i]) {
        bytes.push(byte);
      }
      if (i < refs.length) {
        writeSlebIndex(bytes, numbered[refs[i]]);
      }
    }
    work += bytes.length;
    table.push(bytes);
  }
  return { table, roots: mappedRoots, work };
}
