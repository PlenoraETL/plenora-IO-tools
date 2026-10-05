use crate::{CodePair, DxfError, DxfResult};

use crate::code_pair_put_back::CodePairPutBack;

pub(crate) const EXTENSION_DATA_GROUP: i32 = 102;

/// Quanti gruppi `{ ... }` possono stare uno dentro l'altro, qui e nei gruppi
/// di controllo XDATA (`1002`).
///
/// La specifica non fissa un massimo, e un tetto c'e' lo stesso perche' la
/// lettura e' ricorsiva: senza, un ingresso di pochi megabyte -- duecentomila
/// `102/{a` di fila -- esauriva lo stack del thread principale, un abort senza
/// busta misurato il 2026-10-05. Rendere iterativa la sola lettura non
/// basterebbe: il tipo resta un albero, e `Clone`, `Drop`, `PartialEq`,
/// `Debug` e la scrittura (`add_code_pairs`) lo attraversano per ricorsione.
///
/// 256 e' misurato, non stimato. Sul binario di rilascio, nel thread principale
/// di Windows (1 MiB, il piu' piccolo su cui la CLI legge), la lettura
/// completa -- BLOCK, INSERT esploso, entita' -- regge circa 1660 livelli di
/// XDATA e 2340 di gruppi `102`; in una build non ottimizzata circa 415 e 439.
/// Su Linux il thread principale ha 8 MiB. Oltre il tetto la lettura si
/// rifiuta con `ParseError` e ferma l'iteratore. E' un limite dichiarato nel
/// registro del fork, con la condizione per toglierlo.
pub(crate) const MASSIMA_PROFONDITA_DEI_GRUPPI: usize = 256;

/// Represents an application name and a collection of extension group data in the form of `CodePair`s.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtensionGroup {
    pub application_name: String,
    pub items: Vec<ExtensionGroupItem>,
}

/// Represents a single piece of extension data or a named group.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
pub enum ExtensionGroupItem {
    CodePair(CodePair),
    Group(ExtensionGroup),
}

impl ExtensionGroup {
    pub(crate) fn read_group(
        application_name: String,
        iter: &mut CodePairPutBack,
        offset: usize,
    ) -> DxfResult<ExtensionGroup> {
        ExtensionGroup::read_group_a_profondita(application_name, iter, offset, 1)
    }
    fn read_group_a_profondita(
        application_name: String,
        iter: &mut CodePairPutBack,
        offset: usize,
        profondita: usize,
    ) -> DxfResult<ExtensionGroup> {
        if profondita > MASSIMA_PROFONDITA_DEI_GRUPPI {
            return Err(iter.ferma(offset));
        }
        if !application_name.starts_with('{') {
            return Err(DxfError::ParseError(offset));
        }
        let mut application_name = application_name;
        application_name.remove(0);

        let mut items = vec![];
        loop {
            let pair = match iter.next() {
                Some(Ok(pair)) => pair,
                Some(Err(e)) => return Err(e),
                None => return Err(DxfError::UnexpectedEndOfInput),
            };
            if pair.code == EXTENSION_DATA_GROUP {
                let name = pair.assert_string()?;
                if name == "}" {
                    // end of group
                    break;
                } else if name.starts_with('{') {
                    // nested group
                    let sub_group = ExtensionGroup::read_group_a_profondita(
                        name,
                        iter,
                        pair.offset,
                        profondita + 1,
                    )?;
                    items.push(ExtensionGroupItem::Group(sub_group));
                } else {
                    return Err(DxfError::UnexpectedCodePair(
                        pair,
                        String::from("expected an extension start or end pair"),
                    ));
                }
            } else {
                items.push(ExtensionGroupItem::CodePair(pair));
            }
        }
        Ok(ExtensionGroup {
            application_name,
            items,
        })
    }
    pub(crate) fn add_code_pairs(&self, pairs: &mut Vec<CodePair>) {
        if !self.items.is_empty() {
            let mut full_group_name = String::new();
            full_group_name.push('{');
            full_group_name.push_str(&self.application_name);
            pairs.push(CodePair::new_string(EXTENSION_DATA_GROUP, &full_group_name));
            for item in &self.items {
                match item {
                    ExtensionGroupItem::CodePair(pair) => pairs.push(pair.clone()),
                    ExtensionGroupItem::Group(ref group) => group.add_code_pairs(pairs),
                }
            }
            pairs.push(CodePair::new_str(EXTENSION_DATA_GROUP, "}"));
        }
    }
}
