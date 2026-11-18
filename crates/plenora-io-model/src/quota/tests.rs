use super::*;
use crate::budget::{OperationCounter, PipelineBudget, ResourcePool, SourceEntry};

#[test]
fn enumeration_has_unique_names_defaults_and_invalid_values() {
    for (index, rule) in QUOTAS.iter().enumerate() {
        assert!(!QUOTAS[..index].iter().any(|other| other.name == rule.name));
        if let Some(value) = rule.quota.value(&PipelineLimits::default()) {
            assert_eq!(value, rule.default);
        }
        for value in ["0", "-1", "abc", "18446744073709551616"] {
            let e = rule.quota.parse(value).unwrap_err();
            assert_eq!(
                (e.category, e.phase, e.code),
                (
                    ErrorCategory::InvalidConfiguration,
                    ErrorPhase::Validate,
                    IoErrorCode::Generic
                )
            );
            assert!(e.message.contains(rule.name));
        }
        for phase in rule.phases {
            let e = rule.quota.exceeded(*phase);
            assert_eq!(
                (e.category, e.phase, e.code),
                (
                    ErrorCategory::ResourceLimit,
                    *phase,
                    IoErrorCode::LimitExceeded
                )
            );
            assert!(e.message.contains(rule.name));
        }
    }
}

#[test]
fn real_counters_and_pool_use_catalogue_identity() {
    for rule in QUOTAS {
        let quota = rule.quota;
        let mut limits = PipelineLimits::default()
            .with_max_wkb_cell_bytes(if quota == Quota::MemoryBytes { 1 } else { 1024 });
        if quota != Quota::ConcurrentOperations {
            limits = quota.set(limits, 1).unwrap();
        }
        let pool = ResourcePool::builder()
            .concurrent_operations(1)
            .build()
            .unwrap();
        let parts = PipelineBudget::builder()
            .limits(limits)
            .resource_pool(pool)
            .build()
            .unwrap()
            .into_read_parts();
        let (budget, permit, _) = parts.into_components();
        let context = budget.context();
        let error = match quota {
            Quota::Rows => budget.try_lease(OperationCounter::Rows, 2).unwrap_err(),
            Quota::Columns => budget.try_lease(OperationCounter::Columns, 2).unwrap_err(),
            Quota::GeometryComponents => budget
                .try_lease(OperationCounter::GeometryComponents, 2)
                .unwrap_err(),
            Quota::OutputBytes => budget
                .try_lease(OperationCounter::OutputBytes, 2)
                .unwrap_err(),
            Quota::MemoryBytes => context.lease_memory_internal(2).unwrap_err(),
            Quota::SpillBytes => context.lease_spill(2).unwrap_err(),
            Quota::InputBytes => context
                .note_entry_visited(&SourceEntry::file(b"a", 2, None))
                .unwrap_err(),
            Quota::InputEntries => {
                context
                    .note_entry_visited(&SourceEntry::file(b"a", 1, None))
                    .unwrap();
                context
                    .note_entry_visited(&SourceEntry::file(b"b", 1, None))
                    .unwrap_err()
            }
            Quota::ConcurrentOperations => {
                let _lease = context.lease_concurrency().unwrap();
                context.lease_concurrency().unwrap_err()
            }
            Quota::OutputExpansionRatio => {
                context
                    .note_entry_visited(&SourceEntry::file(b"a", 1, None))
                    .unwrap();
                context.observe_input(permit.unwrap()).unwrap();
                budget
                    .try_lease(OperationCounter::OutputBytes, 2)
                    .unwrap_err()
            }
            Quota::Vertices | Quota::WkbComponents | Quota::WkbCellBytes | Quota::WkbDepth => {
                // Valid little-endian LineString with two positions; nested collections
                // exercise depth independently of the component count.
                let mut bytes = vec![1];
                bytes.extend(2_u32.to_le_bytes());
                bytes.extend(2_u32.to_le_bytes());
                bytes.extend([0_u8; 32]);
                if quota == Quota::WkbDepth {
                    for _ in 0..2 {
                        let mut parent = vec![1];
                        parent.extend(7_u32.to_le_bytes());
                        parent.extend(1_u32.to_le_bytes());
                        parent.extend(bytes);
                        bytes = parent;
                    }
                }
                limits.quota_error(
                    crate::wkb::inspect_wkb(&bytes, &limits.wkb_limits()).unwrap_err(),
                    ErrorPhase::Read,
                )
            }
            Quota::DecompressionRatio => quota.check_ratio(2, 1, 1, ErrorPhase::Read).unwrap_err(),
        };
        assert_eq!(error.category, ErrorCategory::ResourceLimit);
        assert!(
            error.message.contains(quota.rule().name),
            "{quota:?}: {error}"
        );
    }
    for pool in [
        ResourcePool::builder().memory_bytes(0),
        ResourcePool::builder().spill_bytes(0),
        ResourcePool::builder().concurrent_operations(0),
    ] {
        assert_eq!(
            pool.build().unwrap_err().category,
            ErrorCategory::InvalidConfiguration
        );
    }
}
