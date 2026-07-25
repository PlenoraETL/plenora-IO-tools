//! `LossReport` — un driver `Approximating` deve popolarlo, mai perdere in
//! silenzio (ADR-IO 5). Aggregato per categoria e **bounded**: conteggi +
//! un numero limitato di esempi, mai una voce per feature.

use std::collections::BTreeMap;

/// Tetto agli esempi diagnostici conservati (nessun accumulo illimitato).
pub const MAX_LOSS_EXAMPLES: usize = 64;

#[derive(Clone, Debug)]
pub struct LossExample {
    pub category: String,
    /// Descrizione strutturale, mai il valore sensibile (es. "layer=X row=12").
    pub context: String,
}

#[derive(Clone, Debug, Default)]
pub struct LossReport {
    /// Aggregati per categoria: (categoria -> conteggio).
    pub counts: BTreeMap<String, u64>,
    /// Esempi diagnostici, limitati a `MAX_LOSS_EXAMPLES`.
    examples: Vec<LossExample>,
}

impl LossReport {
    pub fn record(&mut self, category: &str, n: u64) {
        *self.counts.entry(category.to_owned()).or_default() += n;
    }

    /// Aggiunge un esempio solo finché sotto il tetto (bounded).
    pub fn add_example(&mut self, example: LossExample) {
        if self.examples.len() < MAX_LOSS_EXAMPLES {
            self.examples.push(example);
        }
    }

    pub fn examples(&self) -> &[LossExample] {
        &self.examples
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}
