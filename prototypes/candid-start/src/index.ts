// The framework's public surface, in one place. Everything else under src/
// is reachable but considered internal layout, free to move.
export { c } from "./schema.ts";
export type { Schema, Infer } from "./schema.ts";
export { CandidStartError } from "./errors.ts";
export {
  anonymousPrincipal,
  isPrincipalLike,
  principalFromText,
} from "./principal.ts";
export {
  checkSchema,
  validate,
  DEFAULT_VALIDATION_LIMITS,
  type SchemaCheckIssue,
  type ValidationIssue,
  type ValidationLimits,
  type ValidationResult,
} from "./validate.ts";
export {
  decodeBase64,
  encodeBase64,
  fromWire,
  toWire,
  type DecodeResult,
  type WireValue,
} from "./wire.ts";
export {
  checkMethodName,
  emitServiceDid,
  type DidMethodSpec,
} from "./did.ts";
export {
  escapeAttribute,
  escapeText,
  fragment,
  h,
  jsonScript,
  renderDocument,
  renderToString,
  type Child,
  type Component,
  type Props,
  type VNode,
} from "./html.ts";
export {
  defineRoute,
  loaderInputSchema,
  matchRoute,
  type AnyRoute,
  type Route,
  type RouteConfig,
  type RouteMatch,
  type RouteParams,
} from "./router.ts";
export {
  query,
  update,
  InvokeValidationError,
  type AnyServerFn,
  type RequestContext,
  type ServerFn,
} from "./server-fn.ts";
export {
  createApp,
  HYDRATION_SCRIPT_ID,
  RPC_PREFIX,
  type App,
  type AppConfig,
  type CandidSurface,
  type HttpRequest,
  type HttpResponse,
  type ShellProps,
} from "./canister.ts";
export {
  startDevServer,
  type DevServerHandle,
  type DevServerOptions,
} from "./dev-server.ts";
