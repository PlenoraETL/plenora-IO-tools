//! Limiti condivisi. La validazione WKB per cella è ereditata dal contratto del
//! bordo (Architetture §2.2): 64 MiB/cella, 100k componenti, profondità 64.

#[derive(Clone, Copy, Debug)]
pub struct WkbLimits {
    pub max_cell_bytes: usize,
    pub max_components: usize,
    pub max_depth: usize,
}

impl Default for WkbLimits {
    fn default() -> Self {
        Self {
            max_cell_bytes: 64 * 1024 * 1024,
            max_components: 100_000,
            max_depth: 64,
        }
    }
}

#[cfg(test)]
mod tests;
