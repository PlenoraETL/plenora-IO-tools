use parquet::file::reader::{ChunkReader, Length};
use plenora_io_core::{SourceData, SourceReader};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

#[derive(Clone)]
pub struct ParquetInput {
    source: SourceData,
    file: Option<Arc<std::fs::File>>,
    length: u64,
}
impl ParquetInput {
    pub fn open(source: &SourceData) -> std::io::Result<Self> {
        let reader = source.open()?;
        let file = match reader {
            SourceReader::File(f) => Some(Arc::new(f)),
            SourceReader::Memory(_) => None,
        };
        let length = file
            .as_ref()
            .map_or_else(|| source.length(), |f| f.metadata().map(|m| m.len()))?;
        Ok(Self {
            source: source.clone(),
            file,
            length,
        })
    }
    pub const fn length(&self) -> u64 {
        self.length
    }
    pub fn reader(&self) -> std::io::Result<SourceReader> {
        self.file.as_ref().map_or_else(
            || self.source.open(),
            |f| f.try_clone().map(SourceReader::File),
        )
    }
}
impl Length for ParquetInput {
    fn len(&self) -> u64 {
        self.length
    }
}
impl ChunkReader for ParquetInput {
    type T = SourceReader;
    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        if start > self.length {
            return Err(parquet::errors::ParquetError::EOF(
                "range non valido".to_owned(),
            ));
        }
        let mut reader = self.reader()?;
        reader.seek(SeekFrom::Start(start))?;
        Ok(reader)
    }
    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<bytes::Bytes> {
        if start
            .checked_add(length as u64)
            .is_none_or(|end| end > self.length)
        {
            return Err(parquet::errors::ParquetError::EOF(
                "range non valido".to_owned(),
            ));
        }
        let mut output = vec![0; length];
        self.get_read(start)?.read_exact(&mut output)?;
        Ok(output.into())
    }
}
