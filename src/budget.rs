use crate::{CancellationToken, Limits};
use std::collections::BTreeMap;
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
    deadline: Option<Instant>,
    cancellation: CancellationToken,
    consumed: BTreeMap<&'static str, usize>,
}

impl<'a> Budget<'a> {
    pub(crate) fn new(limits: &'a Limits, cancellation: CancellationToken) -> Self {
        Self {
            limits,
            deadline: monotonic_deadline(limits.deadline_unix_ms),
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
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn checkpoint(&self) -> Result<(), BudgetError> {
        if self.cancellation.is_cancelled() {
            return Err(BudgetError::Cancelled);
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
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

/// Gate input length, decode a raw DTO, then checkpoint — the shared shape of
/// every bounded parse entry point.
///
/// The caller keeps the budget afterwards so validation charges the same
/// counters the decode gate already observed, rather than starting from a
/// fresh allowance.
pub(crate) fn decode_bounded<T>(
    budget: &mut Budget<'_>,
    input_len: usize,
    decode: impl FnOnce() -> Result<T, serde_json::Error>,
) -> Result<T, crate::ContractJsonError> {
    observe_input_bytes(budget, input_len).map_err(crate::ContractJsonError::InvalidContract)?;
    let raw =
        decode().map_err(|error| crate::ContractJsonError::MalformedJson(error.to_string()))?;
    budget
        .checkpoint()
        .map_err(BudgetError::into_contract_error)
        .map_err(crate::ContractJsonError::InvalidContract)?;
    Ok(raw)
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

    fn unix_ms() -> u64 {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

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
