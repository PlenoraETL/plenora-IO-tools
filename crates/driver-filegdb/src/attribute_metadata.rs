use super::{err, Dataset, HashMap, PlenoraIoError, PublicMessage, Result};
use plenora_io_model::domains::{
    self, Code, CodeType, CodedValue, Domain, DomainAttributes, Domains, FieldSubtype, MergePolicy,
    SplitPolicy, Subtype, Subtypes,
};
use std::collections::BTreeMap;

/// Native field types of the domain codes, in both directions.
const CODE_TYPES: [(u32, CodeType); 4] = {
    use gdal::vector::OGRFieldType as T;
    [
        (T::OFTInteger, CodeType::Int32),
        (T::OFTInteger64, CodeType::Int64),
        (T::OFTReal, CodeType::Float64),
        (T::OFTString, CodeType::Utf8),
    ]
};

/// `OGRFieldSubType`; 0 is `OFSTNone`.
const FIELD_SUBTYPES: [(u32, FieldSubtype); 5] = [
    (1, FieldSubtype::Boolean),
    (2, FieldSubtype::Int16),
    (3, FieldSubtype::Float32),
    (4, FieldSubtype::Json),
    (5, FieldSubtype::Uuid),
];

/// `OGRFieldDomainSplitPolicy`.
const SPLIT_POLICIES: [(u32, SplitPolicy); 3] = [
    (0, SplitPolicy::DefaultValue),
    (1, SplitPolicy::Duplicate),
    (2, SplitPolicy::GeometryRatio),
];

/// `OGRFieldDomainMergePolicy`.
const MERGE_POLICIES: [(u32, MergePolicy); 3] = [
    (0, MergePolicy::DefaultValue),
    (1, MergePolicy::Sum),
    (2, MergePolicy::GeometryWeighted),
];

fn to_model<T: Copy>(table: &[(u32, T)], native: u32) -> Option<T> {
    table
        .iter()
        .find(|(value, _)| *value == native)
        .map(|(_, model)| *model)
}

fn to_native<T: PartialEq>(table: &[(u32, T)], model: &T) -> u32 {
    table
        .iter()
        .find(|(_, value)| value == model)
        .map_or(0, |(native, _)| *native)
}

fn parse_code(code_type: CodeType, value: String) -> Option<Code> {
    match code_type {
        CodeType::Int32 => value.parse().ok().map(Code::Int32),
        CodeType::Int64 => value.parse().ok().map(Code::Int64),
        CodeType::Float64 => value.parse().ok().map(Code::Float64),
        CodeType::Utf8 => Some(Code::Utf8(value)),
    }
}

/// A native domain is transported whole or not at all: every native
/// attribute (kind, code type, field subtype, description, split and merge
/// policies, values) maps to the model, or the domain is not transported.
fn transport(native: gdal::domains::FieldDomain) -> Option<(String, Domain)> {
    let code_type = to_model(&CODE_TYPES, native.field_type)?;
    let attributes = DomainAttributes {
        description: native.description,
        field_subtype: match native.field_subtype {
            0 => None,
            other => Some(to_model(&FIELD_SUBTYPES, other)?),
        },
        split_policy: to_model(&SPLIT_POLICIES, native.split_policy)?,
        merge_policy: to_model(&MERGE_POLICIES, native.merge_policy)?,
    };
    let domain = match native.values {
        gdal::domains::FieldDomainValues::Coded(values) => Domain::Coded {
            code_type,
            values: values
                .into_iter()
                .map(|(value, description)| {
                    Some(CodedValue {
                        code: parse_code(code_type, value)?,
                        description,
                    })
                })
                .collect::<Option<_>>()?,
            attributes,
        },
        gdal::domains::FieldDomainValues::Range {
            min,
            max,
            min_inclusive,
            max_inclusive,
        } => Domain::Range {
            min: parse_code(code_type, min)?,
            max: parse_code(code_type, max)?,
            min_inclusive,
            max_inclusive,
            attributes,
        },
        gdal::domains::FieldDomainValues::Unsupported => return None,
    };
    let single = Domains::from([(native.name, domain)]);
    domains::validate_domains(&single).ok()?;
    single.into_iter().next()
}

/// The transported domains of the dataset and how many native domains were
/// not transported (loss `native_domain_not_transported`, also when no
/// field references them).
pub(super) fn domains(ds: &Dataset) -> Result<(Domains, u64)> {
    let native = ds.field_domains().map_err(|_| invalid())?;
    let mut result = Domains::new();
    let mut lost = 0;
    for domain in native {
        match transport(domain) {
            Some((name, value)) => {
                result.insert(name, value);
            }
            None => lost += 1,
        }
    }
    Ok((result, lost))
}

fn invalid() -> PlenoraIoError {
    err(&PublicMessage::Curated(
        "metadati nativi FileGDB non validi",
    ))
}

pub(super) fn write_domains(
    ds: &mut Dataset,
    layers: &[plenora_io_core::WriteLayer],
) -> Result<()> {
    let mut seen = Domains::new();
    for layer in layers {
        let Some(value) = layer.contract.schema.metadata().get(domains::DOMAINS_KEY) else {
            continue;
        };
        for (name, domain) in domains::decode_domains(value)? {
            if let Some(previous) = seen.get(&name) {
                if previous != &domain {
                    return Err(invalid());
                }
                continue;
            }
            seen.insert(name.clone(), domain.clone());
            let scalar = |code: &Code| match code {
                Code::Int32(v) => v.to_string(),
                Code::Int64(v) => v.to_string(),
                Code::Float64(v) => v.to_string(),
                Code::Utf8(v) => v.clone(),
            };
            let attributes = domain.attributes().clone();
            let (code_type, values) = match domain {
                Domain::Coded {
                    code_type, values, ..
                } => (
                    code_type,
                    gdal::domains::FieldDomainValues::Coded(
                        values
                            .into_iter()
                            .map(|v| (scalar(&v.code), v.description))
                            .collect(),
                    ),
                ),
                Domain::Range {
                    min,
                    max,
                    min_inclusive,
                    max_inclusive,
                    ..
                } => (
                    min.code_type(),
                    gdal::domains::FieldDomainValues::Range {
                        min: scalar(&min),
                        max: scalar(&max),
                        min_inclusive,
                        max_inclusive,
                    },
                ),
            };
            let field_type = to_native(&CODE_TYPES, &code_type);
            // Existing definitions are shared between layers; reject conflicts upstream.
            if ds
                .field_domains()
                .map_err(|_| invalid())?
                .iter()
                .any(|d| d.name == name)
            {
                continue;
            }
            let _written = ds
                .add_field_domain(&gdal::domains::FieldDomain {
                    name,
                    description: attributes.description,
                    field_type,
                    field_subtype: attributes
                        .field_subtype
                        .map_or(0, |value| to_native(&FIELD_SUBTYPES, &value)),
                    split_policy: to_native(&SPLIT_POLICIES, &attributes.split_policy),
                    merge_policy: to_native(&MERGE_POLICIES, &attributes.merge_policy),
                    values,
                })
                .map_err(|_| invalid())?;
            // Reopening the staged output compares the actual delivered metadata and
            // reports unsupported native writes through the common loss table.
        }
    }
    Ok(())
}

#[derive(Default)]
struct Node {
    tag: String,
    text: String,
    children: Vec<Self>,
}

impl Node {
    fn child(&self, tag: &str) -> Option<&Self> {
        self.children.iter().find(|n| n.tag == tag)
    }
    fn value(&self, tag: &str) -> Option<&str> {
        self.child(tag).map(|n| n.text.as_str())
    }

    fn aliases(&self, result: &mut BTreeMap<String, String>) {
        if matches!(self.tag.as_str(), "Field" | "GPFieldInfoEx") {
            if let (Some(name), Some(alias)) = (self.value("Name"), self.value("AliasName")) {
                if !alias.is_empty() {
                    result.insert(name.to_owned(), alias.to_owned());
                }
            }
        }
        for child in &self.children {
            child.aliases(result);
        }
    }
}

fn parse(xml: &str) -> Result<Node> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = vec![Node::default()];
    loop {
        let event = reader.read_event().map_err(|_| invalid())?;
        match event {
            Event::Start(tag) => {
                if stack.len() > 128 {
                    return Err(invalid());
                }
                stack.push(Node {
                    tag: tag.local_name().as_ref().to_owned(),
                    ..Node::default()
                });
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(invalid)?;
                stack.last_mut().ok_or_else(invalid)?.children.push(node);
            }
            Event::Text(text) => stack.last_mut().ok_or_else(invalid)?.text.push_str(&text),
            Event::CData(text) => stack.last_mut().ok_or_else(invalid)?.text.push_str(&text),
            Event::GeneralRef(reference) => {
                let text = reference.as_ref();
                let escaped = format!("&{text};");
                stack
                    .last_mut()
                    .ok_or_else(invalid)?
                    .text
                    .push_str(&quick_xml::escape::unescape(&escaped).map_err(|_| invalid())?);
            }
            Event::Eof if stack.len() == 1 => return stack.pop().ok_or_else(invalid),
            Event::Eof | Event::DocType(_) => return Err(invalid()),
            _ => {}
        }
    }
}

pub(super) fn definition(xml: &str) -> Result<HashMap<String, String>> {
    let root = parse(xml)?;
    let root = root.children.first().ok_or_else(invalid)?;
    let mut metadata = HashMap::new();
    if let Some(alias) = root.value("AliasName").filter(|s| !s.is_empty()) {
        metadata.insert("plenora.layer.alias".to_owned(), alias.to_owned());
    }
    if let Some(list) = root.child("Subtypes").filter(|n| !n.children.is_empty()) {
        let field = root
            .value("SubtypeFieldName")
            .ok_or_else(invalid)?
            .to_owned();
        let default = root
            .value("DefaultSubtypeCode")
            .map(str::parse)
            .transpose()
            .map_err(|_| invalid())?;
        let mut values = Vec::new();
        for subtype in &list.children {
            if subtype.tag != "Subtype" {
                continue;
            }
            let mut domains = BTreeMap::new();
            if let Some(infos) = subtype.child("FieldInfos") {
                for info in &infos.children {
                    if let Some(domain) = info.value("DomainName").filter(|s| !s.is_empty()) {
                        domains.insert(
                            info.value("FieldName").ok_or_else(invalid)?.to_owned(),
                            domain.to_owned(),
                        );
                    }
                }
            }
            values.push(Subtype {
                code: subtype
                    .value("SubtypeCode")
                    .ok_or_else(invalid)?
                    .parse()
                    .map_err(|_| invalid())?,
                name: subtype.value("SubtypeName").ok_or_else(invalid)?.to_owned(),
                domains,
            });
        }
        metadata.insert(
            domains::SUBTYPES_KEY.to_owned(),
            domains::encode_subtypes(&Subtypes {
                field,
                default,
                values,
            })?,
        );
    }
    Ok(metadata)
}

pub(super) fn xml_text(fragment: &str) -> Result<String> {
    let root = parse(&format!("<value>{fragment}</value>"))?;
    let value = root.children.into_iter().next().ok_or_else(invalid)?;
    if !value.children.is_empty() {
        return Err(invalid());
    }
    Ok(value.text)
}

pub(super) fn aliases(xml: &str) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    parse(xml)?.aliases(&mut result);
    Ok(result)
}

#[cfg(test)]
mod transport_tests {
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
}
