// A hermetic stand-in so the golden type-check needs no npm dependency beyond
// the compiler itself: the goldens import only the *type* Principal, and this
// stub is what tsconfig paths resolve "@dfinity/principal" to. The runtime
// package is a consumer concern, deliberately outside this crate.
export interface Principal {
  toText(): string;
}
