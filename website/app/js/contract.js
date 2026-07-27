// Reading a canonical Contract.
//
// Everything below works off the document candid-core emits and the provenance
// sidecar that accompanies it. Nothing here re-derives Candid semantics: the
// type graph arrives already canonical, already validated, and already
// identified, and this file only walks it.

/** `candid-core:interface:v1:sha256:58f5e2e6…8a10c5` for display. */
export function shortId(id) {
  if (!id) return "—";
  const digest = id.slice(id.lastIndexOf(":") + 1);
  return `${digest.slice(0, 8)}…${digest.slice(-6)}`;
}

/**
 * Indexes the provenance sidecar so the renderer can put names back on the
 * labels the canonical graph reduced to Candid hashes.
 *
 * The canonical Contract keeps method names — a service cannot be called
 * without them — but record and variant labels survive only as their
 * `idl_hash`. Names for those live in `SourceInfo.field_labels`, which is
 * provenance, not identity: dropping the sidecar leaves `contract_id` and
 * `interface_id` byte-identical, and the type graph still means the same
 * thing. It just reads worse.
 */
function indexLabels(sourceInfo) {
  const fields = new Map();
  const args = new Map();
  for (const label of sourceInfo?.field_labels ?? []) {
    if (label.label?.kind === "named") {
      fields.set(`${label.container}:${label.id}`, label.label.name);
    }
  }
  for (const argument of sourceInfo?.function_arguments ?? []) {
    args.set(`${argument.function}:${argument.direction}:${argument.position}`, argument.name);
  }
  return { fields, args };
}

/**
 * Everything the page needs about one compiled version, derived once.
 *
 * @param {object} result The bridge's success response.
 */
export function contractView(result) {
  const contract = JSON.parse(result.contract_document);
  const labels = indexLabels(result.source_info);
  const names = new Map();
  for (const declaration of contract.declarations ?? []) {
    if (!names.has(declaration.type)) names.set(declaration.type, declaration.name);
  }
  const view = {
    contract,
    labels,
    names,
    sourceInfo: result.source_info,
    contractId: contract.identities.contract,
    interfaceId: contract.identities.interface ?? null,
    sourceBundleId: result.source_info?.source_bundle_id ?? null,
    artifactId: result.artifact_id,
    artifactBytes: result.artifact_bytes,
    producer: contract.producer,
    typeCount: contract.types.length,
  };
  view.methods = actorMethods(view);
  view.declarations = (contract.declarations ?? []).map((declaration) => ({
    name: declaration.name,
    text: renderType(view, declaration.type, { expand: true }),
    source: result.source_info?.declarations?.find((entry) => entry.name === declaration.name)?.source,
  }));
  return view;
}

/**
 * A structural fingerprint of the type at `index`, independent of node
 * numbering, declaration names, and which document it came from.
 *
 * Two fingerprints are equal exactly when the two types are the same Candid
 * type, which is what makes "did this method's signature move between v1.2.0
 * and v2.0.0?" answerable across two separately compiled contracts. Recursion
 * is closed the usual way: a back edge is emitted as its distance up the
 * current path rather than followed.
 */
export function fingerprint(view, index, path = []) {
  const seen = path.indexOf(index);
  if (seen !== -1) return `rec^${path.length - seen}`;
  const node = view.contract.types[index];
  const next = [...path, index];
  const of = (child) => fingerprint(view, child, next);
  switch (node.kind) {
    case "primitive":
      return node.primitive;
    case "opt":
      return `opt(${of(node.inner)})`;
    case "vec":
      return `vec(${of(node.inner)})`;
    case "record":
    case "variant": {
      const fields = [...node.fields]
        .sort((a, b) => a.id - b.id)
        .map((field) => `${field.id}:${of(field.type)}`);
      return `${node.kind}(${fields.join(",")})`;
    }
    case "func":
      return `func:${node.mode}(${node.args.map(of).join(",")})->(${node.results.map(of).join(",")})`;
    case "service": {
      const methods = [...node.methods]
        .sort((a, b) => a.id - b.id)
        .map((method) => `${method.id}:${of(method.function)}`);
      return `service(${methods.join(",")})`;
    }
    case "class":
      return `class(${node.init.map(of).join(",")};${of(node.service)})`;
    default:
      return `unknown:${node.kind}`;
  }
}

/**
 * Renders the type at `index` back to Candid-ish source.
 *
 * A declaration name short-circuits the walk, which is what keeps the output
 * readable and what makes it terminate on a self-referential type.
 */
export function renderType(view, index, { expand = false, path = [] } = {}) {
  if (!expand && view.names.has(index)) return view.names.get(index);
  if (path.includes(index)) return view.names.get(index) ?? "…";

  const node = view.contract.types[index];
  const next = [...path, index];
  const of = (child) => renderType(view, child, { path: next });

  switch (node.kind) {
    case "primitive":
      return node.primitive;
    case "opt":
      return `opt ${of(node.inner)}`;
    case "vec":
      return `vec ${of(node.inner)}`;
    case "record": {
      // Candid tuple syntax: consecutive labels from 0 with no names of their
      // own. `record { text; text }` compiles to exactly this.
      const tuple = node.fields.every(
        (field, position) => field.id === position && !view.labels.fields.has(`${index}:${field.id}`),
      );
      const fields = node.fields.map((field) => {
        const name = view.labels.fields.get(`${index}:${field.id}`);
        return tuple ? of(field.type) : `${name ?? field.id} : ${of(field.type)}`;
      });
      return fields.length === 0 ? "record {}" : `record { ${fields.join("; ")} }`;
    }
    case "variant": {
      const fields = node.fields.map((field) => {
        const name = view.labels.fields.get(`${index}:${field.id}`) ?? String(field.id);
        const payload = view.contract.types[field.type];
        // `Foo` and `Foo : null` are the same case; Candid spells the common
        // one without a payload.
        if (payload.kind === "primitive" && payload.primitive === "null") return name;
        return `${name} : ${of(field.type)}`;
      });
      return fields.length === 0 ? "variant {}" : `variant { ${fields.join("; ")} }`;
    }
    case "func":
      return `func ${renderSignature(view, index)}`;
    case "service":
      return `service { ${node.methods
        .map((method) => `${method.name} : ${renderSignature(view, method.function)}`)
        .join("; ")} }`;
    case "class":
      return `(${node.init.map(of).join(", ")}) -> ${of(node.service)}`;
    default:
      return node.kind;
  }
}

/** `(TransferArgs) -> (TransferResult) query`, argument names included. */
export function renderSignature(view, functionIndex) {
  const node = view.contract.types[functionIndex];
  const render = (child) => renderType(view, child, { path: [functionIndex] });
  const named = (direction, position, child) => {
    const name = view.labels.args.get(`${functionIndex}:${direction}:${position}`);
    return name ? `${name} : ${render(child)}` : render(child);
  };
  const args = node.args.map((child, position) => named("argument", position, child));
  const results = node.results.map((child, position) => named("result", position, child));
  const mode = node.mode === "update" ? "" : ` ${node.mode}`;
  return `(${args.join(", ")}) -> (${results.join(", ")})${mode}`;
}

/**
 * The actor's methods, each with the structural fingerprint that decides
 * cross-version compatibility.
 *
 * Empty for a declaration-only Contract, which is also the case where
 * `interface_id` is absent.
 */
function actorMethods(view) {
  const actor = view.contract.actor;
  if (!actor) return [];
  const service =
    actor.kind === "service"
      ? view.contract.types[actor.service]
      : view.contract.types[view.contract.types[actor.class].service];
  return service.methods
    .map((method) => ({
      name: method.name,
      id: method.id,
      mode: view.contract.types[method.function].mode,
      signature: renderSignature(view, method.function),
      fingerprint: fingerprint(view, method.function),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * How a client written against `from` fares when it meets `to`.
 *
 * `identical` is decided by `interface_id` alone, which is the whole point of
 * having one: a single string comparison over the canonical actor graph,
 * holding across reformatting, comments, renamed files, moved declarations,
 * and reordered members. The method-level detail below it only needs computing
 * when the two identities differ.
 */
export function compareInterfaces(from, to) {
  const before = new Map(from.methods.map((method) => [method.name, method]));
  const after = new Map(to.methods.map((method) => [method.name, method]));
  const added = [...after.keys()].filter((name) => !before.has(name)).sort();
  const removed = [...before.keys()].filter((name) => !after.has(name)).sort();
  const changed = [...before.keys()]
    .filter((name) => after.has(name) && after.get(name).fingerprint !== before.get(name).fingerprint)
    .sort();

  let verdict = "compatible";
  if (from.interfaceId !== null && from.interfaceId === to.interfaceId) {
    verdict = "identical";
  } else if (removed.length > 0 || changed.length > 0) {
    verdict = "breaking";
  }
  return { verdict, added, removed, changed };
}
