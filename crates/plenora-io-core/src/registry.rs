//! Registro dei driver + dispatch per id (Architetture §5.1).

use crate::descriptor::FormatDescriptor;
use crate::driver::FormatDriver;

#[derive(Default)]
pub struct DriverRegistry {
    drivers: Vec<Box<dyn FormatDriver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, driver: Box<dyn FormatDriver>) {
        self.drivers.push(driver);
    }

    pub fn descriptors(&self) -> Vec<&FormatDescriptor> {
        self.drivers.iter().map(|d| d.descriptor()).collect()
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn FormatDriver> {
        self.drivers
            .iter()
            .find(|d| d.descriptor().id == id)
            .map(|b| b.as_ref())
    }
}
