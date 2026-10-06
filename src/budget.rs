use crate::{CancellationToken, Limits};
use std::collections::BTreeMap;
use std::ops::Range;
#[cfg(not(target_os = "unknown"))]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BudgetError {
    Cancelled,
    DeadlineExceeded,
    ResourceLimit {
        resource: &'static str,
        limit: usize,
        observed: usize,
    },
}

pub(crate) struct Budget<'a> {
    limits: &'a Limits,
    deadline: Deadline,
    cancellation: CancellationToken,
    consumed: BTreeMap<&'static str, usize>,
}

impl<'a> Budget<'a> {
    pub(crate) fn new(limits: &'a Limits, cancellation: CancellationToken) -> Self {
        Self {
            limits,
            deadline: Deadline::snapshot(limits.deadline_unix_ms),
            cancellation,
            consumed: BTreeMap::new(),
        }
    }

    pub(crate) fn from_limits(limits: &'a Limits) -> Self {
        Self::new(limits, CancellationToken::new())
    }

    pub(crate) fn limits(&self) -> &'a Limits {
        self.limits
    }

    /// Nested contexts must inherit this token; constructing a fresh one would
    /// silently make the caller's cancellation unobservable to resolvers.
    /// Only provenance rederivation needs this: it builds a nested context
    /// that must observe the caller's cancellation token.
    #[cfg(feature = "compiler")]
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn checkpoint(&self) -> Result<(), BudgetError> {
        if self.cancellation.is_cancelled() {
            return Err(BudgetError::Cancelled);
        }
        if self.deadline.exceeded() {
            return Err(BudgetError::DeadlineExceeded);
        }
        Ok(())
    }

    pub(crate) fn charge(
        &mut self,
        resource: &'static str,
        limit: usize,
        amount: usize,
    ) -> Result<usize, BudgetError> {
        self.checkpoint()?;
        let consumed = self.consumed.entry(resource).or_default();
        let observed = consumed.saturating_add(amount);
        if observed > limit {
            return Err(BudgetError::ResourceLimit {
                resource,
                limit,
                observed,
            });
        }
        *consumed = observed;
        Ok(observed)
    }

    /// Records a retained-resource high-water mark without double-counting the
    /// same artifact when later stages revalidate it.
    pub(crate) fn observe(
        &mut self,
        resource: &'static str,
        limit: usize,
        observed: usize,
    ) -> Result<usize, BudgetError> {
        self.checkpoint()?;
        let consumed = self.consumed.entry(resource).or_default();
        let high_water = (*consumed).max(observed);
        if high_water > limit {
            return Err(BudgetError::ResourceLimit {
                resource,
                limit,
                observed: high_water,
            });
        }
        *consumed = high_water;
        Ok(high_water)
    }

    /// Read a running total back. HostValue validation preflights child
    /// counts with it; the budget tests assert against it in every
    /// configuration.
    #[cfg(any(feature = "host-value", test))]
    pub(crate) fn consumed(&self, resource: &'static str) -> usize {
        self.consumed.get(resource).copied().unwrap_or_default()
    }
}

/// Reject an oversized document before any decode allocates, then record the
/// length as a high-water observation.
///
/// Every bounded parse entry point shares this so `input_bytes` metadata is
/// emitted from one place.
///
/// `observe` rather than `charge` is deliberate, though no current call site
/// records `input_bytes` twice on one budget. `input_bytes` describes a
/// retained artifact — the document itself — not incremental work, so the
/// high-water mark is the semantically correct counter, and it stays correct
/// if a future nested parse ever re-observes the same input. Using `charge`
/// here would make that future stacking reject documents that are within their
/// limit: the regression #57 fixed for `sources` and `import_edges`, noted in
/// `crate::source`.
pub(crate) fn observe_input_bytes(
    budget: &mut Budget<'_>,
    input_len: usize,
) -> Result<(), crate::ContractValidationError> {
    let limit = budget.limits().max_input_bytes;
    if input_len > limit {
        return Err(crate::ContractValidationError::resource_limit(
            "input_bytes",
            limit,
            input_len,
        ));
    }
    budget
        .observe("input_bytes", limit, input_len)
        .map(|_| ())
        .map_err(BudgetError::into_contract_error)
}

/// Gate input length, decode a raw DTO, refuse serde's alternate forms (a
/// struct written as a JSON array, a unit variant written as a map), then
/// checkpoint — the shared shape of every bounded parse entry point.
///
/// `opaque_root_keys` names the root keys whose values are free-form JSON
/// rather than part of the closed format (the envelope's `extensions`); see
/// [`refuse_alternate_forms`].
///
/// The caller keeps the budget afterwards so validation charges the same
/// counters the decode gate already observed, rather than starting from a
/// fresh allowance.
pub(crate) fn decode_bounded<T>(
    budget: &mut Budget<'_>,
    input: &[u8],
    opaque_root_keys: &[&str],
    decode: impl FnOnce() -> Result<T, serde_json::Error>,
) -> Result<T, crate::ContractJsonError> {
    observe_input_bytes(budget, input.len()).map_err(crate::ContractJsonError::InvalidContract)?;
    let raw =
        decode().map_err(|error| crate::ContractJsonError::MalformedJson(error.to_string()))?;
    refuse_alternate_forms(input, opaque_root_keys)
        .map_err(crate::ContractJsonError::MalformedJson)?;
    budget
        .checkpoint()
        .map_err(BudgetError::into_contract_error)
        .map_err(crate::ContractJsonError::InvalidContract)?;
    Ok(raw)
}

/// The keys whose value is a JSON array in a document the bounded loaders
/// read: the Contract's (`types`, `declarations`, a record's or variant's
/// `fields`, a service's `methods`, a func's `args` and `results`, a class's
/// `init`) and the `SourceInfo` sidecar's (`sources`, `imports`,
/// `declarations`, `field_labels`, `methods`, `function_arguments`, `actors`,
/// and `docs`). Every other value in those formats is an object or a scalar,
/// and no array holds an array.
const ARRAY_KEYS: &[&str] = &[
    "types",
    "declarations",
    "fields",
    "methods",
    "args",
    "results",
    "init",
    "sources",
    "imports",
    "field_labels",
    "function_arguments",
    "actors",
    "docs",
];

/// The keys whose value is a JSON object in a document the bounded loaders
/// read: the Contract's (`identities`, `producer`, `actor`), the envelope's
/// (`contract`, `extensions`), the Compilation's (`contract`, `source_info`)
/// and the `SourceInfo` sidecar's (an entry's `origin` and `label`). Every
/// other object in those formats is the document itself or an element of one
/// of [`ARRAY_KEYS`]; every other key holds a string, a number, or an array.
const OBJECT_KEYS: &[&str] = &[
    "identities",
    "producer",
    "actor",
    "contract",
    "extensions",
    "source_info",
    "origin",
    "label",
];

enum Frame {
    /// An object; `key` is where the raw bytes of the key whose value comes
    /// next (or came last) sit in the input, between its quotes.
    Object {
        key: Range<usize>,
        awaiting_key: bool,
    },
    /// An array, at element `index`.
    Array { index: usize },
}

/// Refuse the two spellings serde's derive reads that the format does not
/// have: a struct written as a JSON array (issue #235), and a unit variant
/// written as a JSON object (issue #238).
///
/// Serde's derived `Deserialize` accepts a struct written as an array of its
/// field values in declaration order, and so does an internally tagged enum
/// (`["primitive", "nat"]` for a type node), at every depth, because the
/// buffered content of a tagged enum is replayed through serde's own content
/// deserializer. It also accepts a unit variant of an externally tagged enum
/// (a primitive type, a func's `mode`, an import's `kind`, an argument's
/// `direction`) written as a map from the variant's name to a unit:
/// `{"nat": null}`, and, inside a tagged enum's buffered content,
/// `{"nat": {}}`. The Contract format writes every struct as an object and
/// every unit variant as a string: candid-core writes nothing else, the DTOs
/// deny unknown keys, and `@candid-core/schema`'s `schemaFromContract`
/// refuses both forms. So the bounded loaders refuse them too, with the error
/// a value of the wrong type gets.
///
/// This runs only on a document the typed decode has just accepted, which
/// fixes its shape: it is well-formed JSON, every key is one its DTO names,
/// the only arrays in it are the values of [`ARRAY_KEYS`] or a struct (or a
/// tagged enum) in sequence form, and the only objects in it are the
/// document, the elements of those arrays, the values of [`OBJECT_KEYS`], or
/// a unit variant in map form (serde reads no other string, number, or array
/// position from a map). The rule is therefore positional and knows no
/// struct:
///
/// - an array is allowed only as the value of one of [`ARRAY_KEYS`]; one at
///   the root, one inside another array, or one at any other key is refused;
/// - an object is allowed only at the root, inside an array, or as the value
///   of one of [`OBJECT_KEYS`]; one at any other key is refused.
///
/// A format change that adds an array-valued or object-valued key fails every
/// document that uses it until the key is listed here; one that adds a unit
/// enum at a new key is covered without editing either list.
///
/// The values of `opaque_root_keys` (the envelope's `extensions`, which are
/// arbitrary JSON) are skipped. Documents that fail the typed decode keep the
/// decode's own error, exactly as before.
///
/// One forward pass with an explicit stack (ADR 0005): no recursion. It
/// allocates one frame per open object or array, plus a short-lived string for
/// each key written with an escape, which it decodes as serde does.
fn refuse_alternate_forms(input: &[u8], opaque_root_keys: &[&str]) -> Result<(), String> {
    let mut stack: Vec<Frame> = Vec::new();
    // Depth inside an opaque value; zero outside one.
    let mut opaque_depth = 0usize;
    let mut at = 0usize;
    while at < input.len() {
        match input[at] {
            b'"' => {
                let end = string_end(input, at);
                if opaque_depth == 0 {
                    if let Some(Frame::Object { key, awaiting_key }) = stack.last_mut() {
                        if *awaiting_key {
                            *key = at + 1..end;
                            *awaiting_key = false;
                        }
                    }
                }
                at = end;
            }
            open @ (b'{' | b'[') => {
                if opaque_depth > 0 {
                    opaque_depth += 1;
                } else if let [Frame::Object { key, .. }] = stack.as_slice() {
                    if opaque_root_keys
                        .iter()
                        .any(|name| key_is(input, key.clone(), name))
                    {
                        opaque_depth = 1;
                    }
                }
                if opaque_depth == 0 {
                    if open == b'{' {
                        if !object_allowed(input, stack.last()) {
                            return Err(wrong_type_error(
                                "map, expected a string",
                                input,
                                &stack,
                                at,
                            ));
                        }
                        stack.push(Frame::Object {
                            key: 0..0,
                            awaiting_key: true,
                        });
                    } else if array_allowed(input, stack.last()) {
                        stack.push(Frame::Array { index: 0 });
                    } else {
                        return Err(wrong_type_error(
                            "sequence, expected an object",
                            input,
                            &stack,
                            at,
                        ));
                    }
                }
            }
            b'}' | b']' => {
                if opaque_depth > 0 {
                    opaque_depth -= 1;
                } else {
                    stack.pop();
                }
            }
            b',' if opaque_depth == 0 => match stack.last_mut() {
                Some(Frame::Object { awaiting_key, .. }) => *awaiting_key = true,
                Some(Frame::Array { index, .. }) => *index += 1,
                None => {}
            },
            _ => {}
        }
        at += 1;
    }
    Ok(())
}

/// Whether an array may stand in `parent` (`None`: at the root): only as the
/// value of one of [`ARRAY_KEYS`], never at the root or inside an array.
fn array_allowed(input: &[u8], parent: Option<&Frame>) -> bool {
    match parent {
        Some(Frame::Object { key, .. }) => ARRAY_KEYS
            .iter()
            .any(|name| key_is(input, key.clone(), name)),
        None | Some(Frame::Array { .. }) => false,
    }
}

/// Whether an object may stand in `parent` (`None`: at the root): at the
/// root, inside an array, or as the value of one of [`OBJECT_KEYS`].
fn object_allowed(input: &[u8], parent: Option<&Frame>) -> bool {
    match parent {
        Some(Frame::Object { key, .. }) => OBJECT_KEYS
            .iter()
            .any(|name| key_is(input, key.clone(), name)),
        None | Some(Frame::Array { .. }) => true,
    }
}

/// The index of the quote closing the string whose opening quote is at
/// `start` (the input is well-formed JSON, so it exists).
fn string_end(input: &[u8], start: usize) -> usize {
    let mut at = start + 1;
    while at < input.len() {
        match input[at] {
            b'\\' => at += 2,
            b'"' => return at,
            _ => at += 1,
        }
    }
    input.len()
}

/// Whether the key whose raw bytes sit at `key` in `input` (between its
/// quotes) spells `name`. A key holding an escape is decoded first, so
/// `"typ\u0065s"` is `types`, as serde reads it.
fn key_is(input: &[u8], key: Range<usize>, name: &str) -> bool {
    let raw = &input[key.clone()];
    if !raw.contains(&b'\\') {
        return raw == name.as_bytes();
    }
    decode_key(input, key).is_some_and(|decoded| decoded == name)
}

/// The key whose raw bytes sit at `key` in `input`, decoded as serde decodes
/// it (the quotes around it are part of the input).
fn decode_key(input: &[u8], key: Range<usize>) -> Option<String> {
    let quoted = input.get(key.start.checked_sub(1)?..key.end + 1)?;
    serde_json::from_slice(quoted).ok()
}

/// The error a value of the wrong type gets: the class serde gives one
/// (`invalid type: <what>`), the `$`-rooted path of the value, and serde's
/// line and column of its opening bracket or brace.
fn wrong_type_error(what: &str, input: &[u8], stack: &[Frame], at: usize) -> String {
    let mut path = String::from("$");
    for frame in stack {
        match frame {
            Frame::Object { key, .. } => {
                let name = decode_key(input, key.clone())
                    .unwrap_or_else(|| String::from_utf8_lossy(&input[key.clone()]).into_owned());
                let identifier = name.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                }) && !name.is_empty();
                if identifier {
                    path.push('.');
                    path.push_str(&name);
                } else {
                    path.push('[');
                    path.push_str(&serde_json::Value::String(name).to_string());
                    path.push(']');
                }
            }
            Frame::Array { index, .. } => {
                path.push('[');
                path.push_str(&index.to_string());
                path.push(']');
            }
        }
    }
    let before = &input[..at];
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = at
        - before
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |n| n + 1)
        + 1;
    format!("invalid type: {what} at {path}, line {line} column {column}")
}

impl BudgetError {
    pub(crate) fn into_contract_error(self) -> crate::ContractValidationError {
        match self {
            Self::Cancelled => crate::ContractValidationError::single(
                "operation_cancelled",
                "$",
                "operation was cancelled",
            ),
            Self::DeadlineExceeded => crate::ContractValidationError::single(
                "operation_deadline_exceeded",
                "$",
                "operation deadline has elapsed",
            ),
            Self::ResourceLimit {
                resource,
                limit,
                observed,
            } => crate::ContractValidationError::resource_limit(resource, limit, observed),
        }
    }
}

/// The configured deadline, snapshotted once when a budget is created.
///
/// A deadline is configured in Unix milliseconds but enforced against a
/// monotonic clock, so wall-clock adjustments cannot extend or shorten an
/// operation mid-flight.
///
/// Bare `wasm32-unknown-unknown` (`target_os = "unknown"`) supplies no clock at
/// all: `SystemTime::now` and `Instant::now` panic there rather than returning
/// an error, so a deadline cannot be measured. An explicit deadline must not
/// become unbounded and must not abort the process either, so on that target
/// the snapshot records only *whether* one was configured and reports it as
/// already elapsed — the fail-closed direction, reaching callers through the
/// same `operation_deadline_exceeded` result an elapsed native deadline
/// produces. `None` stays unbounded exactly as it does natively, and neither
/// cancellation nor any quantitative limit is affected.
#[derive(Clone, Copy)]
struct Deadline {
    #[cfg(not(target_os = "unknown"))]
    instant: Option<Instant>,
    #[cfg(target_os = "unknown")]
    configured: bool,
}

impl Deadline {
    #[cfg(not(target_os = "unknown"))]
    fn snapshot(deadline_unix_ms: Option<u64>) -> Self {
        Self {
            instant: monotonic_deadline(deadline_unix_ms),
        }
    }

    #[cfg(target_os = "unknown")]
    fn snapshot(deadline_unix_ms: Option<u64>) -> Self {
        Self {
            configured: deadline_unix_ms.is_some(),
        }
    }

    #[cfg(not(target_os = "unknown"))]
    fn exceeded(&self) -> bool {
        self.instant
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    #[cfg(target_os = "unknown")]
    fn exceeded(&self) -> bool {
        self.configured
    }
}

#[cfg(not(target_os = "unknown"))]
fn monotonic_deadline(deadline_unix_ms: Option<u64>) -> Option<Instant> {
    let deadline_unix_ms = deadline_unix_ms?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::MAX);
    let now_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
    let remaining_ms = deadline_unix_ms.saturating_sub(now_ms);
    let now = Instant::now();
    Some(
        now.checked_add(Duration::from_millis(remaining_ms))
            // An explicit deadline must never silently become unbounded. If
            // the platform cannot represent it monotonically, fail closed.
            .unwrap_or(now),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "unknown"))]
    fn unix_ms() -> u64 {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

    /// On a clockless target every explicit deadline — past, future, or
    /// unrepresentable — fails closed, and no deadline stays unbounded. The
    /// native cases below cover the measured behaviour.
    #[cfg(target_os = "unknown")]
    #[test]
    fn explicit_deadlines_fail_closed_without_a_clock() {
        for deadline_unix_ms in [0, 1, u64::MAX] {
            let limits = Limits {
                deadline_unix_ms: Some(deadline_unix_ms),
                ..Limits::default()
            };
            assert_eq!(
                Budget::from_limits(&limits).checkpoint(),
                Err(BudgetError::DeadlineExceeded)
            );
        }
        let unbounded = Limits::default();
        assert_eq!(unbounded.deadline_unix_ms, None);
        assert_eq!(Budget::from_limits(&unbounded).checkpoint(), Ok(()));
    }

    #[cfg(not(target_os = "unknown"))]
    #[test]
    fn deadline_is_snapshotted_into_monotonic_time() {
        let future = Limits {
            deadline_unix_ms: Some(unix_ms().saturating_add(60_000)),
            ..Limits::default()
        };
        assert_eq!(Budget::from_limits(&future).checkpoint(), Ok(()));

        let elapsed = Limits {
            deadline_unix_ms: Some(unix_ms().saturating_sub(1)),
            ..Limits::default()
        };
        assert_eq!(
            Budget::from_limits(&elapsed).checkpoint(),
            Err(BudgetError::DeadlineExceeded)
        );
    }

    #[cfg(not(target_os = "unknown"))]
    #[test]
    fn extreme_deadline_is_accepted_when_representable_and_fails_closed_otherwise() {
        let remaining_ms = u64::MAX.saturating_sub(unix_ms());
        let is_representable = Instant::now()
            .checked_add(Duration::from_millis(remaining_ms))
            .is_some();
        let limits = Limits {
            deadline_unix_ms: Some(u64::MAX),
            ..Limits::default()
        };
        let expected = if is_representable {
            Ok(())
        } else {
            Err(BudgetError::DeadlineExceeded)
        };
        assert_eq!(Budget::from_limits(&limits).checkpoint(), expected);
    }

    #[test]
    fn charging_reports_the_attempted_cumulative_value() {
        let limits = Limits::default();
        let mut budget = Budget::from_limits(&limits);
        assert_eq!(budget.charge("test_work", 3, 2), Ok(2));
        assert_eq!(
            budget.charge("test_work", 3, 2),
            Err(BudgetError::ResourceLimit {
                resource: "test_work",
                limit: 3,
                observed: 4,
            })
        );
    }
}

/// The scan on its own, in every feature configuration; the struct positions
/// of each loader's documents are pinned end to end in
/// `tests/struct_sequence_form.rs`.
#[cfg(test)]
mod struct_sequence_tests {
    use super::refuse_alternate_forms;

    fn refused_at(input: &str, opaque: &[&str]) -> Option<String> {
        refuse_alternate_forms(input.as_bytes(), opaque)
            .err()
            .map(|message| {
                let rest = message
                    .strip_prefix("invalid type: sequence, expected an object at ")
                    .expect("the wrong-type class");
                rest[..rest.find(", line ").expect("a line")].to_string()
            })
    }

    #[test]
    fn arrays_stand_only_at_the_formats_list_keys() {
        assert_eq!(
            refused_at(r#"{"types":[{"fields":[]}],"declarations":[]}"#, &[]),
            None
        );
        assert_eq!(refused_at("[]", &[]), Some("$".to_string()));
        assert_eq!(
            refused_at(r#"{"types":[{"fields":[[97,1]]}]}"#, &[]),
            Some("$.types[0].fields[0]".to_string())
        );
        assert_eq!(
            refused_at(r#"{"types":[{},{"kind":"x"},["primitive","nat"]]}"#, &[]),
            Some("$.types[2]".to_string())
        );
        assert_eq!(
            refused_at(r#"{"producer":["a","b","c","d"]}"#, &[]),
            Some("$.producer".to_string())
        );
        assert_eq!(
            refused_at(r#"{"a b":[]}"#, &[]),
            Some(r#"$["a b"]"#.to_string())
        );
    }

    #[test]
    fn keys_are_read_as_serde_reads_them_and_strings_are_text() {
        assert_eq!(refused_at(r#"{"types":[]}"#, &[]), None);
        assert_eq!(
            refused_at(r#"{"actor":"]\"[","actor":[]}"#, &[]),
            Some("$.actor".to_string())
        );
        assert_eq!(refused_at(r#"{"name":"x\"[0]","types":[]}"#, &[]), None);
    }

    #[test]
    fn opaque_root_values_are_skipped() {
        let envelope = r#"{"contract":{"types":[]},"extensions":{"a/v1":[[1],{"b":[[]]}]}}"#;
        assert_eq!(refused_at(envelope, &["extensions"]), None);
        assert_eq!(
            refused_at(envelope, &[]),
            Some(r#"$.extensions["a/v1"]"#.to_string())
        );
        // Only at the root: a nested `extensions` key is not opaque.
        assert_eq!(
            refused_at(r#"{"contract":{"extensions":[]}}"#, &["extensions"]),
            Some("$.contract.extensions".to_string())
        );
        // The scan resumes after the opaque value closes.
        assert_eq!(
            refused_at(r#"{"extensions":{"x":[]},"contract":[]}"#, &["extensions"]),
            Some("$.contract".to_string())
        );
    }

    #[test]
    fn line_and_column_name_the_opening_bracket() {
        let message =
            refuse_alternate_forms(b"{\n  \"identities\": [\"x\"]\n}", &[]).expect_err("refused");
        assert!(
            message.ends_with("at $.identities, line 2 column 17"),
            "{message}"
        );
    }
}

/// The scan's object rule on its own, in every feature configuration; the
/// unit-variant positions of each loader's documents are pinned end to end in
/// `tests/unit_variant_map_form.rs`.
#[cfg(test)]
mod unit_variant_map_tests {
    use super::refuse_alternate_forms;

    fn refused_at(input: &str, opaque: &[&str]) -> Option<String> {
        refuse_alternate_forms(input.as_bytes(), opaque)
            .err()
            .map(|message| {
                let rest = message
                    .strip_prefix("invalid type: map, expected a string at ")
                    .unwrap_or_else(|| panic!("the wrong-type class: {message}"));
                rest[..rest.find(", line ").expect("a line")].to_string()
            })
    }

    #[test]
    fn objects_stand_at_the_root_in_arrays_and_at_the_formats_object_keys() {
        let contract = r#"{"identities":{"contract":"c"},"producer":{"name":"n"},
            "types":[{"kind":"primitive","primitive":"nat"},
                     {"kind":"func","args":[],"results":[],"mode":"query"}],
            "declarations":[{"name":"T","type":0}],"actor":{"kind":"service","service":0}}"#;
        assert_eq!(refused_at(contract, &[]), None);
        let compilation = r#"{"contract":{"types":[]},"source_info":{
            "field_labels":[{"origin":{"kind":"actor","source":"a"},"label":{"kind":"numeric"}}],
            "imports":[{"kind":"type"}],"function_arguments":[{"direction":"result"}]}}"#;
        assert_eq!(refused_at(compilation, &[]), None);
        assert_eq!(refused_at("{}", &[]), None);
    }

    #[test]
    fn an_object_at_any_other_key_is_refused() {
        for (input, path) in [
            (
                r#"{"types":[{"primitive":{"nat":null}}]}"#,
                "$.types[0].primitive",
            ),
            (
                r#"{"types":[{"primitive":{"nat":{}}}]}"#,
                "$.types[0].primitive",
            ),
            (
                r#"{"types":[{},{"mode":{"query":null}}]}"#,
                "$.types[1].mode",
            ),
            (
                r#"{"source_info":{"imports":[{"kind":{"type":null}}]}}"#,
                "$.source_info.imports[0].kind",
            ),
            (
                r#"{"source_info":{"function_arguments":[{"direction":{"argument":null}}]}}"#,
                "$.source_info.function_arguments[0].direction",
            ),
            (r#"{"format":{}}"#, "$.format"),
            (r#"{"a b":{}}"#, r#"$["a b"]"#),
        ] {
            assert_eq!(refused_at(input, &[]), Some(path.to_string()), "{input}");
        }
    }

    #[test]
    fn keys_are_read_as_serde_reads_them() {
        // An escaped object key still holds its object; an escaped unit key
        // still refuses one.
        assert_eq!(refused_at(r#"{"producer":{"name":"n"}}"#, &[]), None);
        assert_eq!(
            refused_at(r#"{"types":[{"mode":{"query":null}}]}"#, &[]),
            Some("$.types[0].mode".to_string())
        );
        // Braces inside strings are text.
        assert_eq!(
            refused_at(r#"{"types":[{"primitive":"{\"nat\":{}}"}]}"#, &[]),
            None
        );
    }

    #[test]
    fn opaque_root_values_are_skipped() {
        let envelope = r#"{"contract":{"types":[]},"extensions":{"a/v1":{"mode":{"x":{}}}}}"#;
        assert_eq!(refused_at(envelope, &["extensions"]), None);
        assert_eq!(
            refused_at(envelope, &[]),
            Some(r#"$.extensions["a/v1"]"#.to_string())
        );
        // The scan resumes after the opaque value closes.
        assert_eq!(
            refused_at(
                r#"{"extensions":{"x":{}},"contract":{"types":[{"mode":{"oneway":null}}]}}"#,
                &["extensions"]
            ),
            Some("$.contract.types[0].mode".to_string())
        );
    }

    #[test]
    fn line_and_column_name_the_opening_brace() {
        let message = refuse_alternate_forms(
            b"{\n  \"types\": [\n    {\"primitive\": {\"nat\": null}}\n  ]\n}",
            &[],
        )
        .expect_err("refused");
        assert_eq!(
            message,
            "invalid type: map, expected a string at $.types[0].primitive, line 3 column 19"
        );
    }
}
