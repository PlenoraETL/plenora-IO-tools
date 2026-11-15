use super::*;
use parquet::data_type::{Int96, Int96Type};
use parquet::file::{
    properties::{EnabledStatistics, WriterProperties},
    writer::SerializedFileWriter,
};
#[test]
fn tempo_remoto_int96_data_and_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let schema = Arc::new(
        parquet::schema::parser::parse_message_type("message test { REQUIRED INT96 time; }")
            .unwrap(),
    );
    for (name, nanos, day) in [
        ("0001", 0u64, 1721426u32),
        ("9999", 0u64, 5373484u32),
        ("nanos-max", 85636854775807, 2547339),
        ("nanos-over", 85636854775808, 2547339),
        ("nanos-min", 763145224192, 2333836),
        ("nanos-under", 763145224191, 2333836),
        ("nanos", u64::MAX, 2440588),
        ("day", 0, u32::MAX),
        ("negative-day", 0, 0x80000000),
        ("full-day", 86400000000000, 2440588),
        ("epoch", 0, 2440588),
    ] {
        let path = dir.path().join(format!("{name}.parquet"));
        let props = Arc::new(
            WriterProperties::builder()
                .set_statistics_enabled(EnabledStatistics::Page)
                .build(),
        );
        let mut writer =
            SerializedFileWriter::new(File::create(&path).unwrap(), schema.clone(), props).unwrap();
        let mut group = writer.next_row_group().unwrap();
        let mut col = group.next_column().unwrap().unwrap();
        let mut value = Int96::new();
        value.set_data(nanos as u32, (nanos >> 32) as u32, day);
        col.typed::<Int96Type>()
            .write_batch(&[value], None, None)
            .unwrap();
        col.close().unwrap();
        group.close().unwrap();
        writer.close().unwrap();
        use parquet::file::reader::FileReader as _;
        let metadata_reader =
            parquet::file::reader::SerializedFileReader::new(File::open(&path).unwrap()).unwrap();
        let statistics = metadata_reader
            .metadata()
            .row_group(0)
            .column(0)
            .statistics()
            .unwrap();
        let parquet::file::statistics::Statistics::Int96(statistics) = statistics else {
            panic!("INT96 statistics expected")
        };
        assert_eq!(statistics.min_opt(), Some(&value));
        assert_eq!(statistics.max_opt(), Some(&value));
        let error = valida_schema_arrow_incorporato(
            File::open(&path).unwrap(),
            std::fs::metadata(&path).unwrap().len(),
            16 * 1024 * 1024,
        )
        .unwrap_err();
        assert!(error.message.contains("INT96"), "{error:?}");
        let public_error = match GeoParquetDriver
            .open(Source::Path(path.clone()), opzioni_lettura())
        {
            Ok(_) => panic!("INT96 must be rejected before projection/pruning can be requested"),
            Err(error) => error,
        };
        assert!(public_error.message.contains("INT96"));
        // Explicit regeneration only; ordinary tests never modify the source tree.
        if std::env::var_os("TEMPO_GENERATE_SEEDS").is_some() {
            std::fs::copy(
                &path,
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "../../fuzz/seeds/geoparquet_reader/tempo-int96-{name}.parquet"
                )),
            )
            .unwrap();
        }
    }
}
