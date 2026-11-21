use super::*;
use gdal::domains::{FieldDomain, FieldDomainValues};

fn native(field_type: u32, values: FieldDomainValues) -> FieldDomain {
    FieldDomain {
        name: "d".into(),
        description: Some("Descrizione è".into()),
        field_type,
        field_subtype: 2,
        split_policy: 1,
        merge_policy: 2,
        values,
    }
}

#[test]
fn every_native_attribute_is_transported_or_the_domain_is_not() {
    use gdal::vector::OGRFieldType as T;
    let coded = || FieldDomainValues::Coded(vec![("1".into(), Some("Uno".into()))]);
    let (_, domain) = transport(native(T::OFTInteger, coded())).expect("trasportato");
    assert_eq!(
        domain.attributes(),
        &DomainAttributes {
            description: Some("Descrizione è".into()),
            field_subtype: Some(FieldSubtype::Int16),
            split_policy: SplitPolicy::Duplicate,
            merge_policy: MergePolicy::GeometryWeighted,
        }
    );
    let range = FieldDomainValues::Range {
        min: "-1.5".into(),
        max: "2".into(),
        min_inclusive: true,
        max_inclusive: false,
    };
    assert!(transport(native(T::OFTReal, range)).is_some());
    let rejected = [
        native(T::OFTDateTime, coded()),
        native(T::OFTInteger, FieldDomainValues::Unsupported),
        native(
            T::OFTInteger,
            FieldDomainValues::Coded(vec![("x".into(), None)]),
        ),
        FieldDomain {
            field_subtype: 99,
            ..native(T::OFTInteger, coded())
        },
        FieldDomain {
            split_policy: 9,
            ..native(T::OFTInteger, coded())
        },
        FieldDomain {
            merge_policy: 9,
            ..native(T::OFTInteger, coded())
        },
        FieldDomain {
            description: Some("a\0b".into()),
            ..native(T::OFTInteger, coded())
        },
    ];
    for domain in rejected {
        assert!(transport(domain.clone()).is_none(), "{domain:?}");
    }
}

#[test]
fn native_tables_are_bijective() {
    fn check<T: Copy + PartialEq + std::fmt::Debug>(table: &[(u32, T)]) {
        for (native, model) in table {
            assert_eq!(to_model(table, *native), Some(*model));
            assert_eq!(to_native(table, model), *native);
        }
    }
    check(&CODE_TYPES);
    check(&FIELD_SUBTYPES);
    check(&SPLIT_POLICIES);
    check(&MERGE_POLICIES);
}
