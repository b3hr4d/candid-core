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
