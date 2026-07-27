// What the frontend needs, and how it decides whether a given wallet can
// provide it.
//
// This is the part that makes a multi-version fleet workable. The page does not
// ask a wallet "what version are you?" and switch on the answer — a version
// string is a claim, and a wallet can be rebuilt, forked, or patched without
// moving it. It asks "does this exact method carry this exact signature?", and
// the answer comes from comparing structural fingerprints taken from the
// canonical Contract of the wallet against those of a reference release.
//
// Each capability lists the shapes the frontend can actually drive. More than
// one shape means the code has a branch for each; the negotiation below picks
// which branch runs.

export const CAPABILITIES = [
  {
    id: "balance",
    label: "Show the balance",
    need: "required",
    detail: "One accept covers both release lines: `balance` has not moved since 1.0.0.",
    accepts: [{ id: "balance/1.0.0", method: "balance", reference: "v1.0.0", lines: ["1.x", "2.x"] }],
    degraded: "The wallet cannot be displayed at all.",
  },
  {
    id: "send",
    label: "Send tokens",
    need: "required",
    detail: "Two accepts, because 2.0.0 changed both the argument and the error type.",
    accepts: [
      { id: "transfer/1.0.0", method: "transfer", reference: "v1.0.0", lines: ["1.x"] },
      { id: "transfer/2.0.0", method: "transfer", reference: "v2.0.0", lines: ["2.x"] },
    ],
    degraded: "Sending is switched off and the wallet is shown read-only.",
  },
  {
    id: "control",
    label: "Name who controls it",
    need: "required",
    detail: "Two accepts under different method names: 2.0.0 replaced `owner` with `controllers`.",
    accepts: [
      { id: "owner/1.0.0", method: "owner", reference: "v1.0.0", lines: ["1.x"] },
      { id: "controllers/2.0.0", method: "controllers", reference: "v2.0.0", lines: ["2.x"] },
    ],
    degraded: "Control of the wallet cannot be attributed.",
  },
  {
    id: "build",
    label: "Report the running build",
    need: "optional",
    detail: "Added in 1.1.0 and unchanged since, so one accept serves every later release.",
    accepts: [
      { id: "wallet_version/1.1.0", method: "wallet_version", reference: "v1.1.0", lines: ["1.x", "2.x"] },
    ],
    degraded: "Shown as “build unknown”.",
  },
  {
    id: "history",
    label: "Page through history",
    need: "optional",
    detail: "Added in 1.2.0 and carried into the 2.x line unchanged.",
    accepts: [
      { id: "transactions/1.2.0", method: "transactions", reference: "v1.2.0", lines: ["1.x", "2.x"] },
    ],
    degraded: "The history tab is hidden.",
  },
  {
    id: "subaccounts",
    label: "Read a subaccount balance",
    need: "optional",
    detail: "2.1.0 only. Needs the `Account` type that arrived with the 2.x line.",
    accepts: [{ id: "balance_of/2.1.0", method: "balance_of", reference: "v2.1.0", lines: ["2.x"] }],
    degraded: "Only the default subaccount is shown.",
  },
  {
    id: "metadata",
    label: "Show token metadata",
    need: "optional",
    detail: "2.1.0 only.",
    accepts: [{ id: "metadata/2.1.0", method: "metadata", reference: "v2.1.0", lines: ["2.x"] }],
    degraded: "Metadata rows are omitted.",
  },
];

/** The release lines this frontend claims to support, toggled in the UI. */
export const ALL_LINES = ["1.x", "2.x"];

/**
 * Decides what the frontend can do against one wallet.
 *
 * A capability is served when some enabled accept names a method the wallet
 * has *and* that method's fingerprint equals the reference release's. A method
 * that exists under the right name with the wrong shape is reported as a
 * mismatch rather than as missing, because those are different bugs: one is a
 * wallet that is too old, the other is a wallet the frontend would crash on if
 * it called it blind.
 */
export function negotiate(release, supportedLines, byId) {
  const capabilities = CAPABILITIES.map((capability) => {
    let fallbackStatus = "missing";
    let mismatch = null;
    for (const accept of capability.accepts) {
      if (!accept.lines.some((line) => supportedLines.includes(line))) continue;
      const reference = byId.get(accept.reference);
      const expected = reference?.methods?.find((method) => method.name === accept.method);
      const actual = release.methods?.find((method) => method.name === accept.method);
      if (!expected || !actual) continue;
      if (actual.fingerprint === expected.fingerprint) {
        return { capability, status: "served", accept, signature: actual.signature };
      }
      fallbackStatus = "mismatch";
      mismatch = { accept, expected, actual };
    }
    return { capability, status: fallbackStatus, mismatch };
  });

  const failed = capabilities.filter((entry) => entry.status !== "served");
  const blocking = failed.filter((entry) => entry.capability.need === "required");
  const status = blocking.length > 0 ? "blocked" : failed.length > 0 ? "reduced" : "full";
  return { capabilities, failed, blocking, status };
}
