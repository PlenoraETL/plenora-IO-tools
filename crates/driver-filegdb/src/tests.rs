//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// Opzioni di lettura sul modello unificato.
///
/// Da S4.d il percorso di lettura vive interamente li': la memoria dei
/// batch e' una `InternalMemoryLease`, che esiste solo dentro un
/// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
/// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
/// Opzioni di scrittura sul modello unificato.
///
/// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
/// `OperationBudget`, che nasce da una costruzione che puo' fallire.
// Il helper serve solo ai test che girano con `gdal-backend`:
// senza la feature non lo chiama nessuno, e clippy ha ragione.
#[cfg(feature = "gdal-backend")]
fn opzioni_scrittura() -> WriteOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

fn opzioni_lettura() -> ReadOptions {
    match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
    }
}

/// Il gemello in scrittura di `open_without_gdal_feature_is_typed`.
///
/// Senza `gdal-backend` anche `create` deve fallire **tipizzato**, non
/// panicare ne' restituire un errore generico: il chiamante deve poter
/// distinguere «formato non disponibile in questo build» da «piano non
/// valido», e le due cose hanno rimedi diversi.
///
/// Il piano dev'essere **valido**: `create` esegue `validate_write` prima
/// del ramo stub, quindi un piano scorretto fallirebbe nella validazione e
/// il test proverebbe un'altra cosa.
///
/// Aggiunto dopo la diagnostica differenziale del checkpoint su `8e64965`,
/// che ha trovato questo ramo mai eseguito.
#[cfg(not(feature = "gdal-backend"))]
#[test]
fn create_without_gdal_feature_is_typed() {
    use driver_common::geometry_field;
    use plenora_io_core::{WriteLayer, WritePlan};
    use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, GeometryType};
    use plenora_io_model::crs::{CrsKind, ResolvedCrs};

    let crs = ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None);
    let schema: std::sync::Arc<arrow_schema::Schema> = std::sync::Arc::new(
        arrow_schema::Schema::new(vec![geometry_field("geometry", "EPSG:3857")]),
    );
    let mut geometria = GeometryColumnContract::wkb_xy(FieldId(0), "geometry", crs, true);
    geometria.set_exact_geometry_types(vec![GeometryType::Point]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "points".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometria),
            },
        }],
    };

    let opzioni = match plenora_io_model::budget::PipelineBudget::builder().build() {
        Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
        Err(errore) => unreachable!("bundle di test non costruibile: {errore:?}"),
    };

    let errore = FileGdbDriver
        .create(Sink::Path("x.gdb".into()), &plan, &opzioni)
        .map(|_| ())
        .expect_err("senza il tier GDB la scrittura non puo' riuscire");

    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Unsupported);
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::Unsupported
    );
    assert!(
        errore.message.contains("--features gdal-backend"),
        "il messaggio deve dire come rimediare: {}",
        errore.message
    );
}

#[test]
fn open_without_gdal_feature_is_typed() {
    // Nel build di default (senza feature) l'apertura fallisce tipizzata.
    let e = FileGdbDriver
        .open(Source::Path("x.gdb".into()), opzioni_lettura())
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(
        e,
        error if error.code == plenora_io_model::IoErrorCode::Unsupported
    ));
}
