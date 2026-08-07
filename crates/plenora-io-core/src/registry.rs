//! Registro dei driver + dispatch per id (Architetture §5.1).

use crate::descriptor::FormatDescriptor;
use crate::driver::FormatDriver;

#[derive(Default)]
pub struct DriverRegistry {
    drivers: Vec<Box<dyn FormatDriver>>,
}

impl DriverRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, driver: Box<dyn FormatDriver>) {
        self.drivers.push(driver);
    }

    #[must_use]
    pub fn descriptors(&self) -> Vec<&FormatDescriptor> {
        let mut descriptors: Vec<_> = self.drivers.iter().map(|d| d.descriptor()).collect();
        descriptors.sort_unstable_by_key(|descriptor| descriptor.id);
        descriptors
    }

    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&dyn FormatDriver> {
        self.drivers
            .iter()
            .find(|d| d.descriptor().id == id)
            .map(std::convert::AsRef::as_ref)
    }
}
