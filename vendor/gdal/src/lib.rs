#![crate_name = "gdal"]
#![crate_type = "lib"]
// Document all features on docs.rs
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![doc(test(attr(deny(warnings), allow(dead_code, unused_variables))))]

//! # GDAL
//! [GDAL](http://gdal.org/) is a translator and processing library for various raster and vector geospatial data formats.
//!
//! This crate provides safe, idiomatic [Rust](http://www.rust-lang.org/) bindings for GDAL.
//!
//! ## Capabilities
//!
//! GDAL is an incredibly powerful library. For a general understanding of its capabilities,
//! a good place to get started is the [GDAL User-oriented documentation](https://gdal.org/user/index.html).
//! These features include:
//!
//! * Opening raster and vector file formats for reading/writing
//! * Translating between file formats
//! * Reading and writing metadata in raster and vector datasets
//! * Accessing raster bands and their metadata
//! * Reading and writing geospatial coordinate system and projection values
//! * Warping (resampling and re-projecting) between coordinate systems
//!
//! ## Usage
//!
//! This crate provides high-level, idiomatic Rust bindings for GDAL.
//! To do that, it uses [`gdal-sys`](gdal-sys) internally, a low-level interface to the GDAL C library, which is generated using [`bindgen`](https://rust-lang.github.io/rust-bindgen/).
//! Using the `gdal-sys` crate directly is normally not needed, but it can be useful in order to call APIs that have not yet been exposed in `gdal`.
//!
//! ## Version support
//!
//! As a general rule, only GDAL versions in Ubuntu LTS-1 (previous LTS version, that is, GDAL 3.4 in 22.04 at this moment) and newer are supported.
//! `gdal-sys` might support earlier versions using the `bindgen` feature flag, but `gdal` does not.
//!
//! Building this crate assumes a compatible version of GDAL is installed with the corresponding header files and shared libraries.
//! This repository includes pre-generated bindings for GDAL 3.4 through 3.12 (see the `gdal-sys/prebuilt-bindings` directory).
//! If you're compiling against another version of GDAL, you can enable the `bindgen` feature flag to have the bindings generated on the fly.
//!
//! ## Show Me Code!
//!
//! To get you started with GDAL (without having to read the whole manual!),
//! take a look at the examples in the [`raster`](raster#example) and [`vector`](vector#example) modules,
//! but for the maximally impatient, here you go:
//!
//! ```rust, no_run
//! use gdal::Dataset;
//! # fn main() -> gdal::errors::Result<()> {
//! let ds = Dataset::open("fixtures/m_3607824_se_17_1_20160620_sub.tif")?;
//! println!("This {} is in '{}' and has {} bands.", ds.driver().long_name(), ds.spatial_ref()?.name().unwrap(), ds.raster_count());
//! # Ok(())
//! # }
//! ```
//! ```text
//! This GeoTIFF is in 'NAD83 / UTM zone 17N' and has 4 bands.
//! ```
//!
//! ## Data Model
//!
//! At the top level, GDAL uses the same data model to access both vector and raster data sets.
//! There are several shared data model constructs at this level, but the first ones to become
//! familiar with are [`Driver`], [`Dataset`], and [`Metadata`].
//! These provide the general access points to [`raster`]- and [`vector`]-specific constructs.
//!
//! ### Driver
//!
//! One of GDAL's major strengths is the vast number of data formats it's able to work with.
//! The GDAL Manual has a full list of available [raster](https://gdal.org/drivers/raster/index.html)
//! and [vector](https://gdal.org/drivers/vector/index.html) drivers.
//!
//! The [`Driver` API][Driver] provides the requisite access points for working GDAL's drivers.
//!
//! ### Dataset
//!
//! [`Dataset`] is the top-level container for accessing all data within a data set, whether raster or vector.
//! Some methods and traits on `Dataset` are shared between raster and vector datasets,
//! and (due to historical reasons) some associated functions are only applicable to one context or the other.
//! The [`raster`] and [`vector`] modules cover these specifics.
//!
//! ### Metadata
//!
//! Metadata in GDAL takes a number of forms, some of which are specific to purpose
//! (e.g. pixel interpretation, spatial reference system),
//! and other more general-purpose (e.g. acquisition date-time). The former will be covered in
//! relevant sections of the [`raster`] and [`vector`] modules, and the general-purpose data model
//! in the [`Metadata`] API.
//!
//! ### Raster Data
//!
//! A raster `Dataset` has a `size` (`cols`,`rows`), an ordered sequence of [`RasterBand`](raster::RasterBand)s, geospatial
//! metadata, and general-purpose [`Metadata`], common to all the bands.
//!
//! Each `RasterBand` contains a buffer of pixels (a.k.a. _cells_), a _no-data_ value, and other metadata.
//!
//! The [`raster`] module covers these concepts in more detail.
//!
//! ### Vector Data
//!
//! A vector `Dataset` contains a sequence of one or more [`Layer`](vector::Layer)s, geospatial metadata,
//! and general-purpose [`Metadata`], common to all the layers.
//! Each `Layer` in turn contains zero or more [`Feature`](vector::Feature)s, each of which contains  a `geometry`
//! and set of fields.
//!
//! The [`vector`] module covers these concepts in more detail.

pub use version::version_info;

pub mod config;
pub mod cpl;
mod dataset;
pub mod domains {
    //! Owned snapshots of GDAL field domains. Borrowed C handles never escape.
    use crate::{
        errors::{GdalError, Result},
        utils::{_string, _string_array},
        Dataset,
    };
    use std::ffi::CString;

    #[derive(Clone, Debug)]
    pub enum FieldDomainValues {
        Coded(Vec<(String, Option<String>)>),
        Range {
            min: String,
            max: String,
            min_inclusive: bool,
            max_inclusive: bool,
        },
        Unsupported,
    }

    #[derive(Clone, Debug)]
    pub struct FieldDomain {
        pub name: String,
        pub description: Option<String>,
        pub field_type: u32,
        /// `OGRFieldSubType`.
        pub field_subtype: u32,
        /// `OGRFieldDomainSplitPolicy`.
        pub split_policy: u32,
        /// `OGRFieldDomainMergePolicy`.
        pub merge_policy: u32,
        pub values: FieldDomainValues,
    }

    fn invalid() -> GdalError {
        GdalError::BadArgument("invalid field domain".into())
    }

    impl Dataset {
        /// Adds a copied domain. The caller retains ownership of the Rust snapshot.
        pub fn add_field_domain(&mut self, domain: &FieldDomain) -> Result<bool> {
            let name = CString::new(domain.name.as_str())?;
            let description = CString::new(domain.description.as_deref().unwrap_or(""))?;
            let handle = match &domain.values {
                FieldDomainValues::Coded(values) => {
                    let strings: Vec<_> = values
                        .iter()
                        .map(|(code, value)| {
                            Ok((
                                CString::new(code.as_str())?,
                                value
                                    .as_ref()
                                    .map(|s| CString::new(s.as_str()))
                                    .transpose()?,
                            ))
                        })
                        .collect::<Result<_>>()?;
                    let mut entries: Vec<_> = strings
                        .iter()
                        .map(|(code, value)| gdal_sys::OGRCodedValue {
                            pszCode: code.as_ptr() as *mut _,
                            pszValue: value
                                .as_ref()
                                .map_or(std::ptr::null_mut(), |s| s.as_ptr() as *mut _),
                        })
                        .collect();
                    entries.push(gdal_sys::OGRCodedValue {
                        pszCode: std::ptr::null_mut(),
                        pszValue: std::ptr::null_mut(),
                    });
                    unsafe {
                        gdal_sys::OGR_CodedFldDomain_Create(
                            name.as_ptr(),
                            description.as_ptr(),
                            domain.field_type,
                            domain.field_subtype,
                            entries.as_ptr(),
                        )
                    }
                }
                FieldDomainValues::Range {
                    min,
                    max,
                    min_inclusive,
                    max_inclusive,
                } => {
                    let scalar = |value: &str| -> Result<gdal_sys::OGRField> {
                        match domain.field_type {
                            gdal_sys::OGRFieldType::OFTInteger => Ok(gdal_sys::OGRField {
                                Integer: value.parse().map_err(|_| invalid())?,
                            }),
                            gdal_sys::OGRFieldType::OFTInteger64 => Ok(gdal_sys::OGRField {
                                Integer64: value.parse().map_err(|_| invalid())?,
                            }),
                            gdal_sys::OGRFieldType::OFTReal => Ok(gdal_sys::OGRField {
                                Real: value.parse().map_err(|_| invalid())?,
                            }),
                            _ => Err(invalid()),
                        }
                    };
                    let min = scalar(min)?;
                    let max = scalar(max)?;
                    unsafe {
                        gdal_sys::OGR_RangeFldDomain_Create(
                            name.as_ptr(),
                            description.as_ptr(),
                            domain.field_type,
                            domain.field_subtype,
                            &min,
                            *min_inclusive,
                            &max,
                            *max_inclusive,
                        )
                    }
                }
                FieldDomainValues::Unsupported => return Ok(false),
            };
            if handle.is_null() {
                return Err(invalid());
            }
            unsafe {
                gdal_sys::OGR_FldDomain_SetSplitPolicy(handle, domain.split_policy);
                gdal_sys::OGR_FldDomain_SetMergePolicy(handle, domain.merge_policy);
            }
            let mut reason = std::ptr::null_mut();
            let result = unsafe {
                gdal_sys::GDALDatasetAddFieldDomain(self.c_dataset(), handle, &mut reason)
            };
            unsafe {
                gdal_sys::OGR_FldDomain_Destroy(handle);
                gdal_sys::VSIFree(reason.cast());
            }
            Ok(result)
        }
        /// GDAL 3.3 API; the fork's minimum GDAL version is 3.4.
        pub fn field_domains(&self) -> Result<Vec<FieldDomain>> {
            let names = unsafe {
                gdal_sys::GDALDatasetGetFieldDomainNames(self.c_dataset(), std::ptr::null_mut())
            };
            if names.is_null() {
                return Ok(Vec::new());
            }
            let owned = _string_array(names);
            unsafe { gdal_sys::CSLDestroy(names) };
            owned
                .into_iter()
                .map(|name| {
                    let c_name = CString::new(name.as_str())?;
                    let handle = unsafe {
                        gdal_sys::GDALDatasetGetFieldDomain(self.c_dataset(), c_name.as_ptr())
                    };
                    if handle.is_null() {
                        return Err(invalid());
                    }
                    let field_type = unsafe { gdal_sys::OGR_FldDomain_GetFieldType(handle) };
                    let field_subtype = unsafe { gdal_sys::OGR_FldDomain_GetFieldSubType(handle) };
                    let split_policy = unsafe { gdal_sys::OGR_FldDomain_GetSplitPolicy(handle) };
                    let merge_policy = unsafe { gdal_sys::OGR_FldDomain_GetMergePolicy(handle) };
                    let description = _string(unsafe { gdal_sys::OGR_FldDomain_GetDescription(handle) })
                        .filter(|value| !value.is_empty());
                    let kind = unsafe { gdal_sys::OGR_FldDomain_GetDomainType(handle) };
                    let values = match kind {
                        gdal_sys::OGRFieldDomainType::OFDT_CODED => {
                            let entries =
                                unsafe { gdal_sys::OGR_CodedFldDomain_GetEnumeration(handle) };
                            if entries.is_null() {
                                return Err(invalid());
                            }
                            let mut values = Vec::new();
                            let mut index = 0;
                            loop {
                                let entry = unsafe { &*entries.add(index) };
                                let Some(code) = _string(entry.pszCode) else {
                                    break;
                                };
                                values.push((code, _string(entry.pszValue)));
                                index += 1;
                            }
                            FieldDomainValues::Coded(values)
                        }
                        gdal_sys::OGRFieldDomainType::OFDT_RANGE => {
                            let mut min_inclusive = false;
                            let mut max_inclusive = false;
                            let min = unsafe {
                                gdal_sys::OGR_RangeFldDomain_GetMin(handle, &mut min_inclusive)
                            };
                            let max = unsafe {
                                gdal_sys::OGR_RangeFldDomain_GetMax(handle, &mut max_inclusive)
                            };
                            if min.is_null() || max.is_null() {
                                return Err(invalid());
                            }
                            let scalar = |value: *const gdal_sys::OGRField| -> Result<String> {
                                if unsafe {
                                    gdal_sys::OGR_RawField_IsUnset(value) != 0
                                        || gdal_sys::OGR_RawField_IsNull(value) != 0
                                } {
                                    return Err(invalid());
                                }
                                unsafe {
                                    match field_type {
                                        gdal_sys::OGRFieldType::OFTInteger => {
                                            Ok((*value).Integer.to_string())
                                        }
                                        gdal_sys::OGRFieldType::OFTInteger64 => {
                                            Ok((*value).Integer64.to_string())
                                        }
                                        gdal_sys::OGRFieldType::OFTReal => {
                                            Ok((*value).Real.to_string())
                                        }
                                        _ => Err(invalid()),
                                    }
                                }
                            };
                            match (scalar(min), scalar(max)) {
                                (Ok(min), Ok(max)) => FieldDomainValues::Range {
                                    min,
                                    max,
                                    min_inclusive,
                                    max_inclusive,
                                },
                                _ => FieldDomainValues::Unsupported,
                            }
                        }
                        _ => FieldDomainValues::Unsupported,
                    };
                    Ok(FieldDomain {
                        name,
                        description,
                        field_type,
                        field_subtype,
                        split_policy,
                        merge_policy,
                        values,
                    })
                })
                .collect()
        }
    }
}
mod driver;
pub mod errors;
mod gcp;
mod gdal_major_object;
mod geo_transform;
mod metadata;
mod options;
pub mod programs;
pub mod raster;
pub mod spatial_ref;
#[cfg(test)]
pub mod test_utils;
#[cfg(any(major_ge_4, all(major_is_3, minor_ge_10)))]
mod thread_safe;
mod utils;
pub mod vector;
pub mod version;
pub mod vsi;

pub use dataset::{Dataset, DatasetCapability};
pub use geo_transform::{GeoTransform, GeoTransformEx};
pub use options::{DatasetOptions, GdalOpenFlags};
#[cfg(any(major_ge_4, all(major_is_3, minor_ge_10)))]
pub use thread_safe::ThreadSafeDataset;

pub use driver::{Driver, DriverManager, DriverType};
pub use gcp::{Gcp, GcpRef};
#[cfg(any(major_ge_4, all(major_is_3, minor_ge_6)))]
pub use gdal_sys::ArrowArrayStream;
pub use metadata::{Metadata, MetadataEntry};

#[cfg(test)]
fn assert_almost_eq(a: f64, b: f64) {
    let f: f64 = a / b;
    assert!(f < 1.00001);
    assert!(f > 0.99999);
}

mod temporal;
