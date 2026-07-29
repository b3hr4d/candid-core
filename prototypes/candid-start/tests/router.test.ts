import { test } from "node:test";
import assert from "node:assert/strict";
import { c } from "../src/schema.ts";
import { h } from "../src/html.ts";
import {
  defineRoute,
  loaderInputSchema,
  matchRoute,
  type RouteParams,
} from "../src/router.ts";
import { shapeOf } from "../src/walk.ts";

const noop = () => h("div", null);

const home = defineRoute({
  name: "home",
  path: "/",
  data: c.bool,
  loader: () => true,
  component: noop,
});

const noteList = defineRoute({
  name: "notes",
  path: "/notes",
  data: c.bool,
  loader: () => true,
  component: noop,
});

const noteDetail = defineRoute({
  name: "note",
  path: "/notes/:id",
  data: c.bool,
  loader: ({ params }) => params.id.length > 0,
  component: noop,
});

const staticNew = defineRoute({
  name: "note_new",
  path: "/notes/new",
  data: c.bool,
  loader: () => true,
  component: noop,
});

const routes = [home, noteList, noteDetail, staticNew];

test("path params are typed from the literal", () => {
  // Compile-time assertions: these lines type-check only if RouteParams
  // extracts exactly { id: string } and {} respectively.
  const params: RouteParams<"/notes/:id"> = { id: "5" };
  assert.equal(params.id, "5");
  const none: RouteParams<"/"> = {};
  assert.deepEqual(none, {});
});

test("matching: exact, param capture, specificity, trailing slash", () => {
  assert.equal(matchRoute(routes, "/")?.route.name, "home");
  assert.equal(matchRoute(routes, "/notes")?.route.name, "notes");
  assert.equal(matchRoute(routes, "/notes/")?.route.name, "notes");
  assert.deepEqual(matchRoute(routes, "/notes/7")?.params, { id: "7" });
  // A static segment beats a param at equal length.
  assert.equal(matchRoute(routes, "/notes/new")?.route.name, "note_new");
  assert.equal(matchRoute(routes, "/nope"), null);
  assert.equal(matchRoute(routes, "/notes/1/extra"), null);
  assert.equal(matchRoute(routes, "notes"), null);
  assert.equal(matchRoute(routes, "/notes//7"), null);
});

test("params are percent-decoded; invalid encodings match nothing", () => {
  assert.deepEqual(matchRoute(routes, "/notes/a%20b")?.params, { id: "a b" });
  assert.equal(matchRoute(routes, "/notes/%zz"), null);
});

test("route definition fails closed on bad shapes", () => {
  const base = { data: c.bool, loader: () => true, component: noop };
  assert.throws(() => defineRoute({ ...base, name: "no spaces", path: "/x" }));
  assert.throws(() => defineRoute({ ...base, name: "x", path: "notes" }));
  assert.throws(() => defineRoute({ ...base, name: "x", path: "/notes/" }));
  assert.throws(() => defineRoute({ ...base, name: "x", path: "/a b" }));
  assert.throws(() =>
    defineRoute({ ...base, name: "x", path: "/:a/:a" as string }),
  );
  assert.throws(() =>
    defineRoute({
      name: "x",
      path: "/x",
      data: c.opt(c.opt(c.text)),
      loader: () => null,
      component: noop,
    }),
  );
});

test("loader input schema: sorted text params or zero-argument", () => {
  assert.equal(loaderInputSchema(home), null);
  const detail = loaderInputSchema(noteDetail);
  assert.notEqual(detail, null);
  if (detail !== null) {
    assert.deepEqual(Object.keys(shapeOf(detail).fields ?? {}), ["id"]);
  }
  const multi = defineRoute({
    name: "multi",
    path: "/a/:zeta/:alpha",
    data: c.bool,
    loader: () => true,
    component: noop,
  });
  const schema = loaderInputSchema(multi);
  if (schema !== null) {
    assert.deepEqual(Object.keys(shapeOf(schema).fields ?? {}), [
      "alpha",
      "zeta",
    ]);
  }
});
