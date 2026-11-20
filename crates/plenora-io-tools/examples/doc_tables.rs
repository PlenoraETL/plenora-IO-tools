//! Esporta le tabelle operative per i blocchi verificati da `check_docset.py`.
use plenora_io_core::loss_table::{self, TransportDirection};
use plenora_io_model::{error_table::ERROR_RULES, quota::QUOTAS};
use serde_json::json;

fn main() {
    let quotas: Vec<_> = QUOTAS
        .iter()
        .map(|q| {
            json!({
                "name": q.name, "flag": q.cli, "default": q.default,
                "unit": q.unit, "scope": q.scope, "phases": q.phases,
            })
        })
        .collect();
    let errors: Vec<_> = ERROR_RULES
        .iter()
        .map(|e| {
            json!({
                "situation": format!("{:?}", e.situation), "category": e.category,
                "phase": e.phase, "code": e.wire_code, "message": e.message,
                "exit": plenora_io_tools::uscita_della_categoria(e.category),
            })
        })
        .collect();
    let losses: Vec<_> = loss_table::DRIVERS
        .iter()
        .map(|driver| {
            let elements: Vec<_> = loss_table::NATIVE_ELEMENTS
                .iter()
                .map(|element| {
                    let rule = |direction| {
                        driver.qualified_native.map(|native| match direction {
                            TransportDirection::Read => {
                                match loss_table::read_applicability(native, *element) {
                                    loss_table::ReadApplicability::Expressible => format!(
                                        "{:?}",
                                        loss_table::native_rule(native, direction, *element)
                                    ),
                                    loss_table::ReadApplicability::NotApplicable {
                                        reason,
                                        reference,
                                    } => format!("NotApplicable: {reason} ({reference})"),
                                }
                            }
                            TransportDirection::Write => format!(
                                "{:?}",
                                loss_table::native_write_transport_rule(native, *element)
                            ),
                        })
                    };
                    json!({"element": format!("{element:?}"),
                "read": rule(TransportDirection::Read), "write": rule(TransportDirection::Write)})
                })
                .collect();
            json!({"driver": driver.driver, "elements": elements})
        })
        .collect();
    println!(
        "{}",
        json!({"quotas": quotas, "errors": errors, "losses": losses, "deadline_ms": plenora_io_model::budget::PipelineLimits::default().duration_ms()})
    );
}
