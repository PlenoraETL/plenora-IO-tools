//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// I vocabolari statici sono distinti, non vuoti e coprono le famiglie.
///
/// `classe_arrow` e `ColType::nome` sono `match` esaustivi: il compilatore
/// garantisce che ogni variante abbia un nome, non che i nomi siano
/// distinti ne' che la mappa sia quella intesa. La misura di copertura
/// differenziale del checkpoint su `effc4ab` li ha trovati **mai
/// esercitati**, ed e' la ragione per cui questo test esiste: un
/// vocabolario che nessuno attraversa e' una tabella di traduzione di cui
/// nessuno ha mai letto una riga.
#[test]
fn i_vocabolari_statici_sono_distinti_e_coprono_le_famiglie() {
    use arrow_schema::{DataType, Field, TimeUnit};
    use std::sync::Arc;

    let campioni: Vec<(DataType, &str)> = vec![
        (DataType::Boolean, "boolean"),
        (DataType::Int8, "signed_integer"),
        (DataType::Int64, "signed_integer"),
        (DataType::UInt8, "unsigned_integer"),
        (DataType::UInt64, "unsigned_integer"),
        (DataType::Float16, "floating"),
        (DataType::Float64, "floating"),
        (DataType::Utf8, "utf8"),
        (DataType::LargeUtf8, "utf8"),
        (DataType::Binary, "binary"),
        (DataType::FixedSizeBinary(4), "binary"),
        (DataType::Date32, "temporal"),
        (DataType::Time64(TimeUnit::Nanosecond), "temporal"),
        (DataType::Timestamp(TimeUnit::Second, None), "temporal"),
        (DataType::Duration(TimeUnit::Second), "temporal"),
        (DataType::Decimal128(10, 2), "decimal"),
        (DataType::Decimal256(40, 2), "decimal"),
        (
            DataType::List(Arc::new(Field::new("item", DataType::Int32, true))),
            "nested",
        ),
        (
            DataType::Struct(vec![Field::new("a", DataType::Int32, true)].into()),
            "nested",
        ),
        // Il ramo di riserva esiste e va raggiunto: se sparisse, un tipo
        // nuovo di arrow non avrebbe piu' nome e il match non
        // compilerebbe — ma finche' c'e', va provato che risponde.
        (DataType::Null, "altro"),
    ];
    for (tipo, atteso) in campioni {
        assert_eq!(classe_arrow(&tipo), atteso, "classe di {tipo:?}");
    }

    let colonne = [
        (ColType::Integer, "integer"),
        (ColType::Number, "number"),
        (ColType::Boolean, "boolean"),
        (ColType::Text, "text"),
    ];
    let mut visti = std::collections::BTreeSet::new();
    for (tipo, atteso) in colonne {
        assert_eq!(tipo.nome(), atteso);
        assert!(visti.insert(tipo.nome()), "nome duplicato: {}", tipo.nome());
    }
    assert_eq!(visti.len(), colonne.len());
}

#[test]
fn incremental_inference_is_monotonic_and_matches_batch_inference() {
    let values = [JsonValue::Null, JsonValue::from(1), JsonValue::from(2.5)];
    let mut accumulator = TypeAccumulator::default();
    accumulator.observe(ObservedValueClass::Null);
    accumulator.observe(ObservedValueClass::Integer);
    assert_eq!(accumulator.column_type(), ColType::Integer);
    accumulator.observe(ObservedValueClass::Number);

    assert_eq!(accumulator.column_type(), ColType::Number);
    assert_eq!(infer_column(values.iter()), ColType::Number);

    accumulator.observe(ObservedValueClass::Boolean);
    assert_eq!(accumulator.column_type(), ColType::Text);
    accumulator.observe(ObservedValueClass::Integer);
    assert_eq!(accumulator.column_type(), ColType::Text);
}

#[test]
fn inferred_builder_preserves_csv_text_and_null_semantics() {
    let mut builder = InferredColumnBuilder::new(ColType::Text);
    builder.append_csv_cell("  value  ").unwrap();
    builder.append_csv_cell("   ").unwrap();
    let array = builder.finish();
    let strings = array.as_any().downcast_ref::<StringArray>().unwrap();

    assert_eq!(strings.value(0), "  value  ");
    assert!(strings.is_null(1));
}

#[test]
fn inference_never_routes_unrepresentable_integer_through_float() {
    let out_of_i64 = [JsonValue::from(u64::MAX)];
    assert_eq!(infer_column(out_of_i64.iter()), ColType::Text);

    let mixed = [JsonValue::from(i64::MAX), JsonValue::from(0.5)];
    assert_eq!(infer_column(mixed.iter()), ColType::Text);

    let integral = [JsonValue::from(i64::MAX), JsonValue::from(i64::MIN)];
    assert_eq!(infer_column(integral.iter()), ColType::Integer);
}

#[test]
fn inferred_builder_rejects_incompatible_non_null_value() {
    let mut builder = InferredColumnBuilder::new(ColType::Integer);
    assert!(matches!(
        builder.append_str("not-an-integer"),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Schema
    ));
}

#[test]
fn json_conversion_never_conflates_unsupported_or_non_finite_with_null() {
    let unsupported: ArrayRef = Arc::new(arrow_array::BinaryArray::from(vec![Some(
        b"payload".as_slice(),
    )]));
    let non_finite: ArrayRef = Arc::new(Float64Array::from(vec![f64::NAN]));
    let absent: ArrayRef = Arc::new(StringArray::from(vec![None::<&str>]));

    assert!(matches!(
        json_from_array(&unsupported, 0),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Unsupported
    ));
    assert!(matches!(
        json_from_array(&non_finite, 0),
        Err(error) if error.code == plenora_io_model::IoErrorCode::Schema
    ));
    assert_eq!(json_from_array(&absent, 0).unwrap(), JsonValue::Null);
}

#[test]
fn cell_string_renders_wide_int64_textually_exact() {
    let wide = (1_i64 << 53) + 1;
    let array: ArrayRef = Arc::new(Int64Array::from(vec![wide]));

    assert_eq!(
        cell_string(&array, 0).unwrap().as_deref(),
        Some("9007199254740993")
    );
}

#[test]
fn cell_string_maps_null_to_none() {
    let array: ArrayRef = Arc::new(Int64Array::from(vec![None::<i64>]));

    assert_eq!(cell_string(&array, 0).unwrap(), None);
}

#[test]
fn cell_string_passes_through_utf8_and_bool() {
    let strings: ArrayRef = Arc::new(StringArray::from(vec!["testo"]));
    let bools: ArrayRef = Arc::new(BooleanArray::from(vec![true]));

    assert_eq!(cell_string(&strings, 0).unwrap().as_deref(), Some("testo"));
    assert_eq!(cell_string(&bools, 0).unwrap().as_deref(), Some("true"));
}

#[test]
fn cell_string_rejects_non_finite_float64() {
    let nan: ArrayRef = Arc::new(Float64Array::from(vec![f64::NAN]));
    let infinite: ArrayRef = Arc::new(Float64Array::from(vec![f64::INFINITY]));

    assert!(cell_string(&nan, 0).is_err());
    assert!(cell_string(&infinite, 0).is_err());
}

#[test]
fn geometry_index_returns_first_of_two_geoarrow_wkb_fields() {
    let schema = Schema::new(vec![
        Field::new("name", DataType::Utf8, true),
        geometry_field("geom_a", OGC_CRS84),
        geometry_field("geom_b", OGC_CRS84),
    ]);

    assert_eq!(geometry_index(&schema), Some(1));
}

#[test]
fn geometry_index_is_none_without_geometry_metadata() {
    let schema = Schema::new(vec![
        Field::new("a", DataType::Int64, true),
        Field::new("b", DataType::Binary, true),
    ]);

    assert_eq!(geometry_index(&schema), None);
}

#[test]
fn geometry_index_ignores_bare_geometry_name_without_extension() {
    let schema = Schema::new(vec![Field::new("geometry", DataType::Binary, true)]);

    assert_eq!(geometry_index(&schema), None);
}

// --- la rappresentazione testuale dei temporali ------------------------

fn testo<A: Array + 'static>(colonna: A) -> String {
    let array: ArrayRef = Arc::new(colonna);
    match json_from_array(&array, 0).expect("la cella temporale si scrive") {
        JsonValue::String(testo) => testo,
        altro => panic!("atteso testo, arrivato {altro:?}"),
    }
}

/// Le date, compresa una **prima** dell'epoca.
///
/// Il giorno negativo non e' un caso di scuola: `div_euclid` e il
/// calendario di Hinnant esistono per lui, e un'implementazione che
/// dividesse per difetto sbagliata darebbe una data plausibile e falsa --
/// il modo peggiore di sbagliare, perche' nessuno la guarda due volte.
#[test]
fn le_date_si_scrivono_in_iso_anche_prima_dell_epoca() {
    assert_eq!(testo(Date32Array::from(vec![0])), "1970-01-01");
    assert_eq!(testo(Date32Array::from(vec![20_468])), "2026-01-15");
    assert_eq!(testo(Date32Array::from(vec![-1])), "1969-12-31");
    assert_eq!(testo(Date32Array::from(vec![-719_162])), "0001-01-01");
    // Un anno bisestile secolare: il 2000 lo e', il 1900 no, ed e' li' che
    // le implementazioni approssimate si dividono.
    assert_eq!(testo(Date32Array::from(vec![11_016])), "2000-02-29");
    // `Date64` sono millisecondi, e si scrive la data.
    assert_eq!(
        testo(Date64Array::from(vec![1_768_435_200_000])),
        "2026-01-15"
    );
}

/// Le cifre della frazione vengono dall'**unita' dichiarata**, non dal
/// valore.
///
/// E' la parte che rende la rappresentazione deterministica: se le cifre
/// dipendessero dal valore, `12:00:00` e `12:00:00.500` sarebbero due forme
/// della stessa colonna, e due file identici nel contenuto avrebbero
/// larghezze diverse.
#[test]
fn le_cifre_della_frazione_vengono_dall_unita_e_non_dal_valore() {
    assert_eq!(testo(Time32SecondArray::from(vec![45_296])), "12:34:56");
    assert_eq!(
        testo(Time32MillisecondArray::from(vec![45_296_000])),
        "12:34:56.000"
    );
    assert_eq!(
        testo(Time32MillisecondArray::from(vec![45_296_500])),
        "12:34:56.500"
    );
    assert_eq!(
        testo(Time64MicrosecondArray::from(vec![45_296_000_007])),
        "12:34:56.000007"
    );
    assert_eq!(
        testo(Time64NanosecondArray::from(vec![45_296_000_000_009])),
        "12:34:56.000000009"
    );
}

/// Il fuso non riproietta: aggiunge la `Z`.
///
/// Arrow conserva un timestamp con fuso come istante UTC, e il fuso e'
/// metadato di presentazione. Riproiettarlo sarebbe una trasformazione, e
/// questo prodotto non trasforma; ometterlo direbbe «ora locale di chissa'
/// dove». Le due colonne portano lo **stesso** istante, e si distinguono
/// per la sola `Z`.
#[test]
fn il_timestamp_con_fuso_si_scrive_in_utc_e_lo_dichiara() {
    let senza = TimestampSecondArray::from(vec![1_768_480_496]);
    let con: TimestampSecondArray =
        TimestampSecondArray::from(vec![1_768_480_496]).with_timezone("Europe/Rome");
    assert_eq!(testo(senza), "2026-01-15T12:34:56");
    assert_eq!(testo(con), "2026-01-15T12:34:56Z");
}

/// `Duration` e `Interval` restano un rifiuto, e il rifiuto nomina la
/// classe.
///
/// Sono nella stessa classe `Temporal` degli istanti e **non** sono
/// istanti: la loro forma testuale e' una scelta di rappresentazione a se'.
/// La sonda esiste perche' il confine sia dichiarato invece che scoperto:
/// il giorno in cui una conversione la chiede, e' questa prova a doversi
/// muovere.
#[test]
fn le_durate_non_sono_istanti_e_restano_un_rifiuto() {
    let durata: ArrayRef = Arc::new(arrow_array::DurationSecondArray::from(vec![90]));
    let Err(errore) = json_from_array(&durata, 0) else {
        panic!("una durata non ha una forma testuale decisa: deve essere rifiutata");
    };
    assert!(
        errore.message.contains("temporal"),
        "il rifiuto deve nominare la classe, arrivato «{}»",
        errore.message
    );
}
