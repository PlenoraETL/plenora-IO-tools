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
    /// Numero massimo di entry visitate dallo scan della sorgente, directory
    /// comprese. Senza questo tetto una directory ostile fa crescere senza
    /// limite la coda dello scan prima ancora che il conteggio dei byte possa
    /// intervenire: i byte si sommano solo sui file, la coda cresce su tutto.
    pub max_input_entries: u64,
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
            // Una directory di migliaia di file legittimi deve passare; uno
            // scan illimitato no. Stesso ordine di grandezza di max_columns.
            max_input_entries: 10_000,
            max_rows: 10_000_000,
            max_columns: 4_096,
            max_vertices: 50_000_000,
            max_output_bytes: 1_073_741_824,
            wkb: WkbLimits::default(),
        }
    }
}

impl Limits {
    /// Limiti effettivi del decoder geometrico: `max_vertices` è un tetto
    /// globale aggiuntivo rispetto al limite WKB più specifico.
    #[must_use]
    pub fn effective_wkb(&self) -> WkbLimits {
        WkbLimits {
            max_components: self.wkb.max_components.min(self.max_vertices),
            ..self.wkb
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_vertex_limit_tightens_wkb_components() {
        let limits = Limits {
            max_vertices: 3,
            wkb: WkbLimits {
                max_components: 10,
                ..WkbLimits::default()
            },
            ..Limits::default()
        };
        assert_eq!(limits.effective_wkb().max_components, 3);
    }
}
