use super::{err, Dataset, HashMap, PlenoraIoError, PublicMessage, Result};
use plenora_io_model::domains::{
    self, Code, CodeType, CodedValue, Domain, Domains, Subtype, Subtypes,
};
use std::collections::BTreeMap;

pub(super) fn domains(ds: &Dataset) -> Result<Domains> {
    let native = ds.field_domains().map_err(|_| invalid())?;
    let mut result = Domains::new();
    for domain in native {
        let code = |value: String| -> Result<Code> {
            use gdal::vector::OGRFieldType as T;
            match domain.field_type {
                T::OFTInteger => value.parse().map(Code::Int32).map_err(|_| invalid()),
                T::OFTInteger64 => value.parse().map(Code::Int64).map_err(|_| invalid()),
                T::OFTReal => value.parse().map(Code::Float64).map_err(|_| invalid()),
                T::OFTString => Ok(Code::Utf8(value)),
                _ => Err(invalid()),
            }
        };
        let value = match domain.values {
            gdal::domains::FieldDomainValues::Coded(values) => Domain::Coded {
                code_type: match domain.field_type {
                    0 => CodeType::Int32,
                    12 => CodeType::Int64,
                    2 => CodeType::Float64,
                    4 => CodeType::Utf8,
                    _ => continue,
                },
                values: values
                    .into_iter()
                    .map(|(value, description)| {
                        Ok(CodedValue {
                            code: code(value)?,
                            description,
                        })
                    })
                    .collect::<Result<_>>()?,
            },
            gdal::domains::FieldDomainValues::Range {
                min,
                max,
                min_inclusive,
                max_inclusive,
            } => Domain::Range {
                min: code(min)?,
                max: code(max)?,
                min_inclusive,
                max_inclusive,
            },
            gdal::domains::FieldDomainValues::Unsupported => continue,
        };
        result.insert(domain.name, value);
    }
    domains::validate_domains(&result)?;
    Ok(result)
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
                Code::Int32(v) => (0, v.to_string()),
                Code::Int64(v) => (12, v.to_string()),
                Code::Float64(v) => (2, v.to_string()),
                Code::Utf8(v) => (4, v.clone()),
            };
            let (field_type, values) = match domain {
                Domain::Coded { code_type, values } => {
                    let field_type = match code_type {
                        CodeType::Int32 => 0,
                        CodeType::Int64 => 12,
                        CodeType::Float64 => 2,
                        CodeType::Utf8 => 4,
                    };
                    (
                        field_type,
                        gdal::domains::FieldDomainValues::Coded(
                            values
                                .into_iter()
                                .map(|v| (scalar(&v.code).1, v.description))
                                .collect(),
                        ),
                    )
                }
                Domain::Range {
                    min,
                    max,
                    min_inclusive,
                    max_inclusive,
                } => {
                    let (field_type, min) = scalar(&min);
                    (
                        field_type,
                        gdal::domains::FieldDomainValues::Range {
                            min,
                            max: scalar(&max).1,
                            min_inclusive,
                            max_inclusive,
                        },
                    )
                }
            };
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
                    field_type,
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
