// A minimal runtime principal. The schema core deliberately depends only on
// the *type* of `Principal` — the structural `{ toText(): string }` surface
// its hermetic stub declares — so a runtime stand-in this small satisfies
// every generated schema. The agent SDK's real `Principal` (base32 text
// format, CRC check, byte representation) arrives with the transport slice
// (issue #104); nothing in this prototype validates principal text, and that
// is a recorded limitation, not an oversight.
import type { Principal } from "@icp-sdk/core/principal";

class TextPrincipal implements Principal {
  readonly #text: string;

  constructor(text: string) {
    this.#text = text;
  }

  toText(): string {
    return this.#text;
  }
}

export function principalFromText(text: string): Principal {
  return new TextPrincipal(text);
}

/** The IC anonymous principal's canonical text form. */
export const anonymousPrincipal: Principal = principalFromText("2vxsx-fae");

// Structural, not branded: any object exposing `toText()` passes, exactly the
// surface the schema core's stub declares. A branded check would reject the
// agent SDK's own Principal instances once the real dependency arrives.
export function isPrincipalLike(value: unknown): value is Principal {
  return (
    typeof value === "object" &&
    value !== null &&
    "toText" in value &&
    typeof (value as { toText: unknown }).toText === "function"
  );
}
