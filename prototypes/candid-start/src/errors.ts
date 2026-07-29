// One error type for the framework's fail-closed paths. `code` is the stable
// machine surface, mirroring candid-core's diagnostics discipline; message
// text is not.
export class CandidStartError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "CandidStartError";
    this.code = code;
  }
}
