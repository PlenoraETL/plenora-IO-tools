//! Module with the definition of the [Writer] that allows writing shapefile
//!
//! # Writer
//!
//! [Writer] is the struct that writes a complete shapefile (_.shp_, _.shx_, _.dbf_).
//!
//! # ShapeWriter
//!
//! The [ShapeWriter] can be used if you only want to write the .shp
//! and .shx files, however since it does not write the .dbf file, it is not recommended.
use std::io::{BufWriter, Seek, SeekFrom, Write};

use super::{header, ShapeType};
use super::{Error, PointZ};
use crate::record::{BBoxZ, EsriShape, RecordHeader};
use std::fs::File;
use std::path::Path;

use crate::reader::ShapeIndex;
use dbase::TableWriterBuilder;

pub(crate) fn f64_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

pub(crate) fn f64_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

/// struct that handles the writing of the .shp
/// and (optionally) the .idx
///
/// The recommended way to create a ShapeWriter by using [ShapeWriter::from_path]
///
/// # Important
///
/// As this writer does not write the _.dbf_, it does not write what is considered
/// a complete (thus valid) shapefile.
pub struct ShapeWriter<T: Write + Seek> {
    shp_dest: T,
    shx_dest: Option<T>,
    header: header::Header,
    rec_num: u32,
}

impl<T: Write + Seek> ShapeWriter<T> {
    /// Creates a writer that can be used to write a new shapefile.
    ///
    /// The `dest` argument is only for the .shp
    pub fn new(shp_dest: T) -> Self {
        Self {
            shp_dest,
            shx_dest: None,
            header: header::Header::default(),
            rec_num: 1,
        }
    }

    pub fn with_shx(shp_dest: T, shx_dest: T) -> Self {
        Self {
            shp_dest,
            shx_dest: Some(shx_dest),
            header: Default::default(),
            rec_num: 1,
        }
    }

    /// Write the shape to the file
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), shapefile::Error> {
    /// use shapefile::Point;
    /// let mut writer = shapefile::ShapeWriter::from_path("points.shp")?;
    ///
    /// writer.write_shape(&Point::new(0.0, 0.0))?;
    /// writer.write_shape(&Point::new(1.0, 0.0))?;
    /// writer.write_shape(&Point::new(2.0, 0.0))?;
    ///
    /// # std::fs::remove_file("points.shp")?;
    /// # std::fs::remove_file("points.shx")?;
    /// # Ok(())
    /// # }
    /// ```
    /// Reserves the header space, once, before the first record.
    ///
    /// Split out of `write_shape` by the plenora fork: a null record also has
    /// to reserve the header, and it must do so *without* adopting a shape
    /// type. Keying the reservation on `rec_num` instead of on the shape-type
    /// sentinel keeps the two decisions apart, so a file whose first record is
    /// null does not get its header written twice.
    fn reserve_header(&mut self) -> Result<(), Error> {
        if self.rec_num != 1 {
            return Ok(());
        }
        use std::f64::{MAX, MIN};
        self.header.bbox = BBoxZ {
            max: PointZ::new(MIN, MIN, MIN, MIN),
            min: PointZ::new(MAX, MAX, MAX, MAX),
        };
        self.header.write_to(&mut self.shp_dest)?;
        if let Some(shx_dest) = &mut self.shx_dest {
            self.header.write_to(shx_dest)?;
        }
        Ok(())
    }

    /// Writes a null shape record, keeping the file's declared shape type.
    ///
    /// Added by the plenora fork. The shapefile specification allows a record
    /// whose shape type is Null (0) inside a file whose header declares another
    /// type: it is how a feature without geometry is stored, and it is what
    /// every other implementation writes. The upstream API cannot express it —
    /// `Shape::NullShape` carries no value and implements neither
    /// `WritableShape` nor `EsriShape` — so a reader could read such a file and
    /// a writer could not reproduce it.
    ///
    /// This is deliberately **not** a generic "write a raw record" entry point:
    /// it writes one shape, the null one, with the record size the format
    /// fixes for it.
    pub fn write_null_shape(&mut self) -> Result<(), Error> {
        self.reserve_header()?;

        // A null record's content is its shape type and nothing else: four
        // bytes, which the record header counts as two 16-bit words.
        let record_size = (std::mem::size_of::<i32>() / 2) as i32;

        RecordHeader {
            record_number: self.rec_num as i32,
            record_size,
        }
        .write_to(&mut self.shp_dest)?;
        ShapeType::NullShape.write_to(&mut self.shp_dest)?;

        if let Some(shx_dest) = &mut self.shx_dest {
            ShapeIndex {
                offset: self.header.file_length,
                record_size,
            }
            .write_to(shx_dest)?;
        }

        // The bbox is left alone: a null shape has no coordinates, and growing
        // the bounding box from nothing would move it.
        self.header.file_length += record_size + RecordHeader::SIZE as i32 / 2;
        self.rec_num += 1;

        Ok(())
    }

    pub fn write_shape<S: EsriShape>(&mut self, shape: &S) -> Result<(), Error> {
        match (self.header.shape_type, S::shapetype()) {
            // This is the first call to write a non-null shape: the file adopts
            // its type. The header itself is reserved by `reserve_header`,
            // which a preceding null record may already have done.
            (ShapeType::NullShape, t) => {
                self.reserve_header()?;
                self.header.shape_type = t;
            }
            (t1, t2) if t1 != t2 => {
                return Err(Error::MismatchShapeType {
                    requested: t1,
                    actual: t2,
                });
            }
            _ => {}
        }

        let record_size = (shape.size_in_bytes() + std::mem::size_of::<i32>()) / 2;

        RecordHeader {
            record_number: self.rec_num as i32,
            record_size: record_size as i32,
        }
        .write_to(&mut self.shp_dest)?;
        self.header.shape_type.write_to(&mut self.shp_dest)?;
        shape.write_to(&mut self.shp_dest)?;

        if let Some(shx_dest) = &mut self.shx_dest {
            ShapeIndex {
                offset: self.header.file_length,
                record_size: record_size as i32,
            }
            .write_to(shx_dest)?;
        }

        self.header.file_length += record_size as i32 + RecordHeader::SIZE as i32 / 2;
        self.header.bbox.grow_from_shape(shape);
        self.rec_num += 1;

        Ok(())
    }

    /// Writes a collection of shapes to the file
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), shapefile::Error> {
    /// use shapefile::Point;
    /// let mut writer = shapefile::ShapeWriter::from_path("points.shp")?;
    /// let points = vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(2.0, 0.0)];
    ///
    /// writer.write_shapes(&points)?;
    /// # std::fs::remove_file("points.shp")?;
    /// # std::fs::remove_file("points.shx")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ```
    /// # fn main() -> Result<(), shapefile::Error> {
    /// use shapefile::{Point, Polyline};
    /// let mut writer = shapefile::ShapeWriter::from_path("polylines.shp")?;
    /// let points = vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(2.0, 0.0)];
    /// let polyline = Polyline::new(points);
    ///
    /// writer.write_shapes(&vec![polyline])?;
    /// # std::fs::remove_file("polylines.shp")?;
    /// # std::fs::remove_file("polylines.shx")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn write_shapes<'a, S: EsriShape + 'a, C: IntoIterator<Item = &'a S>>(
        mut self,
        container: C,
    ) -> Result<(), Error> {
        for shape in container {
            self.write_shape(shape)?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), Error> {
        if self.header.bbox.max.m == std::f64::MIN && self.header.bbox.min.m == std::f64::MAX {
            self.header.bbox.max.m = 0.0;
            self.header.bbox.min.m = 0.0;
        }

        if self.header.bbox.max.z == std::f64::MIN && self.header.bbox.min.z == std::f64::MAX {
            self.header.bbox.max.z = 0.0;
            self.header.bbox.min.z = 0.0;
        }

        self.shp_dest.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut self.shp_dest)?;
        self.shp_dest.seek(SeekFrom::End(0))?;
        if let Some(shx_dest) = &mut self.shx_dest {
            let mut shx_header = self.header;
            shx_header.file_length = header::HEADER_SIZE / 2
                + ((self.rec_num - 1) as i32 * 2 * std::mem::size_of::<i32>() as i32 / 2);
            shx_dest.seek(SeekFrom::Start(0))?;
            shx_header.write_to(shx_dest)?;
            shx_dest.seek(SeekFrom::End(0))?;
        }
        Ok(())
    }
}

impl<T: Write + Seek> Drop for ShapeWriter<T> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl ShapeWriter<BufWriter<File>> {
    /// Creates a new writer from a path.
    /// Creates both a .shp and .shx files
    ///
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let writer = shapefile::ShapeWriter::from_path("new_file.shp");
    /// ```
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let shp_path = path.as_ref().to_path_buf();
        let shx_path = shp_path.with_extension("shx");

        let shp_file = BufWriter::new(File::create(shp_path)?);
        let shx_file = BufWriter::new(File::create(shx_path)?);

        Ok(Self::with_shx(shp_file, shx_file))
    }
}

/// The Writer writes a complete shapefile that is, it
/// writes the 3 mandatory files (.shp, .shx, .dbf)
///
/// The recommended way to create a new shapefile is via the
/// [Writer::from_path] or [Writer::from_path_with_info] associated functions.
///
/// # Examples
///
/// To create a Writer that writes a .dbf file that has the same
/// structure as .dbf read earlier you will have to do:
///
/// ```
/// # fn main() -> Result<(), shapefile::Error> {
/// let mut reader = shapefile::Reader::from_path("tests/data/multipatch.shp")?;
/// let shape_records = reader.read()?;
/// let table_info = reader.into_table_info();
///
/// let writer = shapefile::Writer::from_path_with_info("new_multipatch.shp", table_info);
///
/// # std::fs::remove_file("new_multipatch.shp")?;
/// # std::fs::remove_file("new_multipatch.shx")?;
/// # std::fs::remove_file("new_multipatch.dbf")?;
/// # Ok(())
/// # }
/// ```
pub struct Writer<T: Write + Seek> {
    shape_writer: ShapeWriter<T>,
    dbase_writer: dbase::TableWriter<T>,
}

impl<T: Write + Seek> Writer<T> {
    /// Creates a new writer using the provided ShapeWriter and TableWriter
    ///
    /// # Example
    ///
    /// Creating a Writer that writes to in memory buffers.
    ///
    /// ```
    /// # fn main() -> Result<(), shapefile::Error> {
    /// use std::convert::TryInto;
    /// let mut shp_dest = std::io::Cursor::new(Vec::<u8>::new());
    /// let mut shx_dest = std::io::Cursor::new(Vec::<u8>::new());
    /// let mut dbf_dest = std::io::Cursor::new(Vec::<u8>::new());
    ///
    /// let shape_writer = shapefile::ShapeWriter::with_shx(&mut shp_dest, &mut shx_dest);
    /// let dbase_writer = dbase::TableWriterBuilder::new()
    ///     .add_character_field("Name".try_into().unwrap(), 50)
    ///     .build_with_dest(&mut dbf_dest);
    ///
    /// let shape_writer = shapefile::Writer::new(shape_writer, dbase_writer);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(shape_writer: ShapeWriter<T>, dbase_writer: dbase::TableWriter<T>) -> Self {
        Self {
            shape_writer,
            dbase_writer,
        }
    }

    pub fn write_shape_and_record<S: EsriShape, R: dbase::WritableRecord>(
        &mut self,
        shape: &S,
        record: &R,
    ) -> Result<(), Error> {
        self.shape_writer.write_shape(shape)?;
        self.dbase_writer.write_record(record)?;
        Ok(())
    }

    /// Writes a null shape and its attribute record.
    ///
    /// Added by the plenora fork, for the same reason as
    /// [`ShapeWriter::write_null_shape`]: a feature without geometry still has
    /// attributes, and the .shp and the .dbf must keep the same number of
    /// records in the same order. Dropping the row instead would misalign them.
    pub fn write_null_shape_and_record<R: dbase::WritableRecord>(
        &mut self,
        record: &R,
    ) -> Result<(), Error> {
        self.shape_writer.write_null_shape()?;
        self.dbase_writer.write_record(record)?;
        Ok(())
    }

    pub fn write_shapes_and_records<
        'a,
        S: EsriShape + 'a,
        R: dbase::WritableRecord + 'a,
        C: IntoIterator<Item = (&'a S, &'a R)>,
    >(
        mut self,
        container: C,
    ) -> Result<(), Error> {
        for (shape, record) in container.into_iter() {
            self.write_shape_and_record(shape, record)?;
        }
        Ok(())
    }
}

impl Writer<BufWriter<File>> {
    /// Creates all the files needed for the shapefile to be complete (.shp, .shx, .dbf)
    ///
    /// ```
    /// # fn main() -> Result<(), shapefile::Error> {
    /// use std::convert::TryInto;
    /// let table_builder = dbase::TableWriterBuilder::new()
    ///     .add_character_field("name".try_into().unwrap(), 50);
    /// let writer = shapefile::Writer::from_path("new_cities.shp", table_builder)?;
    /// # std::fs::remove_file("new_cities.shp")?;
    /// # std::fs::remove_file("new_cities.shx")?;
    /// # std::fs::remove_file("new_cities.dbf")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_path<P: AsRef<Path>>(
        path: P,
        table_builder: TableWriterBuilder,
    ) -> Result<Self, Error> {
        Ok(Self {
            shape_writer: ShapeWriter::from_path(path.as_ref())?,
            dbase_writer: table_builder
                .build_with_file_dest(path.as_ref().with_extension("dbf"))?,
        })
    }

    pub fn from_path_with_info<P: AsRef<Path>>(
        path: P,
        table_info: dbase::TableInfo,
    ) -> Result<Self, Error> {
        Ok(Self {
            shape_writer: ShapeWriter::from_path(path.as_ref())?,
            dbase_writer: dbase::TableWriterBuilder::from_table_info(table_info)
                .build_with_file_dest(path.as_ref().with_extension("dbf"))?,
        })
    }
}
