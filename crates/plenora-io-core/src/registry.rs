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

    /// Il registro delle `format_options`, per id di driver (L0.7, S6).
    ///
    /// E' **derivato** dall'elenco dei driver registrati, non una tabella
    /// scritta accanto: non esiste una riga da dimenticare quando si aggiunge
    /// un driver, ne' una che sopravviva a uno rimosso. Ordinato per id come
    /// `descriptors`, cosi' chi lo serializza ottiene lo stesso ordine a ogni
    /// esecuzione.
    ///
    /// Uno schema vuoto e' un'affermazione — quel driver non interpreta nulla
    /// — e compare nel registro come gli altri.
    #[must_use]
    pub fn format_options(
        &self,
    ) -> Vec<(
        &'static str,
        plenora_io_model::format_options::SchemaOpzioniFormato,
    )> {
        self.descriptors()
            .into_iter()
            .map(|descriptor| (descriptor.id, descriptor.format_options))
            .collect()
    }

    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&dyn FormatDriver> {
        self.drivers
            .iter()
            .find(|d| d.descriptor().id == id)
            .map(std::convert::AsRef::as_ref)
    }
}
