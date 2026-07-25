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

/// Limiti dati/runtime del bordo I/O (bounded-production, CLI-overridable).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_rows: usize,
    pub max_columns: usize,
    pub max_vertices: usize,
    pub max_output_bytes: u64,
    pub wkb: WkbLimits,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 268_435_456,
            max_rows: 10_000_000,
            max_columns: 4_096,
            max_vertices: 50_000_000,
            max_output_bytes: 1_073_741_824,
            wkb: WkbLimits::default(),
        }
    }
}
