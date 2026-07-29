// The example: a notes app in one canister.
//
// Everything a TanStack Start app splits between a server and a bundle lives
// here in one place: the domain schemas are generated from notes.did, the
// server functions are candid methods, the routes SSR through the loaders,
// and `createApp` derives the canister's `.did` from what this file
// registers. The in-memory Map *is* the database — on the IC, canister heap
// survives between calls, which is exactly the single-canister premise.
import type { Principal } from "@icp-sdk/core/principal";
import { c } from "../../src/schema.ts";
import { createApp, type ShellProps } from "../../src/canister.ts";
import { h, type VNode } from "../../src/html.ts";
import { anonymousPrincipal } from "../../src/principal.ts";
import { defineRoute } from "../../src/router.ts";
import { query, update } from "../../src/server-fn.ts";
import { NewNote, Note, NoteId, NoteStatus, PublishResult } from "./schemas.ts";

// -- State ------------------------------------------------------------------

const notes = new Map<bigint, Note>();
let nextId = 1n;

// Deterministic time: the prototype has no ic0.time, and a wall clock would
// make SSR output differ between runs, which the tests pin. A real host
// binding supplies canister time here.
let clockNs = 1_700_000_000_000_000_000n;
function nowNs(): bigint {
  clockNs += 1_000_000_000n;
  return clockNs;
}

function seed(title: string, body: string, tags: string[], owner: Principal): Note {
  const note: Note = {
    id: nextId,
    title,
    body,
    tags,
    created_at_ns: nowNs(),
    owner,
    status: { tag: "draft" },
    attachment: null,
  };
  notes.set(note.id, note);
  nextId += 1n;
  return note;
}

// -- Server functions: candid methods with schemas --------------------------

export const listNotes = query({
  name: "list_notes",
  output: c.vec(Note),
  handler: () => [...notes.values()],
});

export const getNote = query({
  name: "get_note",
  input: NoteId,
  output: c.opt(Note),
  handler: (id) => notes.get(id) ?? null,
});

export const addNote = update({
  name: "add_note",
  input: NewNote,
  output: Note,
  handler: (draft, ctx) => {
    const note: Note = {
      id: nextId,
      title: draft.title,
      body: draft.body,
      tags: draft.tags,
      created_at_ns: nowNs(),
      owner: ctx.caller,
      status: { tag: "draft" },
      attachment: draft.attachment,
    };
    notes.set(note.id, note);
    nextId += 1n;
    return note;
  },
});

export const publishNote = update({
  name: "publish_note",
  input: NoteId,
  output: PublishResult,
  handler: (id) => {
    const note = notes.get(id);
    if (note === undefined) {
      return { tag: "not_found" } as const;
    }
    if (note.status.tag === "published") {
      return { tag: "already_published" } as const;
    }
    const published: Note = {
      ...note,
      status: { tag: "published", value: { at_ns: nowNs() } },
    };
    notes.set(id, published);
    return { tag: "ok", value: published } as const;
  },
});

// -- Components -------------------------------------------------------------

function StatusBadge(props: { readonly status: NoteStatus }): VNode {
  switch (props.status.tag) {
    case "draft":
      return h("span", { class: "badge draft" }, "draft");
    case "archived":
      return h("span", { class: "badge archived" }, "archived");
    case "published":
      return h(
        "span",
        { class: "badge published" },
        "published at ",
        props.status.value.at_ns,
        "ns",
      );
  }
}

function NoteCard(props: { readonly note: Note }): VNode {
  const { note } = props;
  return h(
    "article",
    { class: "note" },
    h("h2", null, h("a", { href: `/notes/${note.id}` }, note.title)),
    h(StatusBadge, { status: note.status }),
    h("p", null, note.body),
    h(
      "ul",
      { class: "tags" },
      note.tags.map((tag) => h("li", null, tag)),
    ),
    h("footer", null, "by ", note.owner.toText()),
  );
}

// -- Routes: loaders are query server functions -----------------------------

export const homeRoute = defineRoute({
  name: "home",
  path: "/",
  data: c.vec(Note),
  // Isomorphic composition: the loader calls the same server function the
  // candid surface exposes, validation included.
  loader: ({ ctx }) => listNotes.invoke(undefined, ctx),
  component: ({ data }) =>
    h(
      "main",
      null,
      h("h1", null, "Notes"),
      data.length === 0
        ? h("p", null, "Nothing here yet.")
        : data.map((note) => h(NoteCard, { note })),
    ),
});

export const noteRoute = defineRoute({
  name: "note",
  path: "/notes/:id",
  data: c.opt(Note),
  loader: ({ params, ctx }) => {
    let id: bigint;
    try {
      id = BigInt(params.id);
    } catch {
      return null;
    }
    if (id < 0n) {
      return null;
    }
    return getNote.invoke(id, ctx);
  },
  component: ({ params, data }) =>
    data === null
      ? h(
          "main",
          null,
          h("h1", null, "Note not found"),
          h("p", null, "No note has id ", params.id, "."),
          h("p", null, h("a", { href: "/" }, "Back to all notes")),
        )
      : h(
          "main",
          null,
          h(NoteCard, { note: data }),
          data.attachment === null
            ? null
            : h("p", null, `Attachment: ${data.attachment.byteLength} bytes`),
          h("p", null, h("a", { href: "/" }, "Back to all notes")),
        ),
});

// -- The canister -----------------------------------------------------------

// Simple selectors only: renderToString escapes text children, so a `>`
// combinator would be entity-escaped inside <style>.
const STYLE = [
  "body { font-family: system-ui, sans-serif; margin: 2rem auto; max-width: 40rem; }",
  ".note { border: 1px solid #ccc; border-radius: 8px; padding: 1rem; margin: 1rem 0; }",
  ".badge { font-size: 0.8rem; padding: 0.1rem 0.5rem; border-radius: 999px; background: #eee; }",
  ".badge.published { background: #cfc; }",
  ".tags li { display: inline; margin-right: 0.5rem; font-size: 0.9rem; color: #555; }",
].join("\n");

function shell(props: ShellProps): VNode {
  return h(
    "html",
    { lang: "en" },
    h(
      "head",
      null,
      h("meta", { charset: "utf-8" }),
      h("meta", { name: "viewport", content: "width=device-width, initial-scale=1" }),
      h("title", null, "candid-start notes"),
      h("style", null, STYLE),
    ),
    h("body", null, h("div", { id: "app" }, props.body), props.scripts),
  );
}

export function createNotesApp(options?: { readonly seedData?: boolean }) {
  notes.clear();
  nextId = 1n;
  clockNs = 1_700_000_000_000_000_000n;
  if (options?.seedData !== false) {
    const owner = anonymousPrincipal;
    seed(
      "Canister-side rendering",
      "The whole app — routes, loaders, server functions — lives in one canister.",
      ["ssr", "candid"],
      owner,
    );
    seed(
      "Schemas are generated",
      "notes.did is compiled by candid-core and emitted as zod-style builders.",
      ["codegen"],
      owner,
    );
  }
  return createApp({
    routes: [homeRoute, noteRoute],
    serverFns: [listNotes, getNote, addNote, publishNote],
    shell,
  });
}
