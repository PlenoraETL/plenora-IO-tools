//! Limiti condivisi.
//!
//! La validazione WKB per cella è ereditata dal contratto del bordo
//! (Architetture §2.2): 64 MiB/cella, 100k componenti; la profondità è quella
//! di Arrow Geometry Semantics 1.0 (GEO-011), 32 livelli.

/// I livelli di annidamento di un valore WKB o EWKB ben formato (GEO-011).
///
/// La geometria esterna sta al livello 0, e un membro al livello 33 rende il
/// valore malformato, come in `arrow_data.py` (`depth > MAX_DEPTH`).
///
/// Non e' una quota: e' parte della **buona forma**. Il parser la applica
/// qualunque sia [`WkbLimits::max_depth`], che puo' solo stringerla; una
/// configurazione piu' larga non fa passare un valore che il contratto
/// rifiuta, e `PipelineLimits::validate` la rifiuta prima.
pub const PROFONDITA_WKB_CONTRATTO: usize = 32;

#[derive(Clone, Copy, Debug)]
pub struct WkbLimits {
    pub max_cell_bytes: usize,
    pub max_components: usize,
    /// Tetto di annidamento richiesto; quello effettivo e' il minimo fra questo
    /// e [`PROFONDITA_WKB_CONTRATTO`].
    pub max_depth: usize,
}

impl Default for WkbLimits {
    fn default() -> Self {
        Self {
            max_cell_bytes: usize::try_from(crate::quota::Quota::WkbCellBytes.rule().default)
                .unwrap_or(usize::MAX),
            max_components: usize::try_from(crate::quota::Quota::WkbComponents.rule().default)
                .unwrap_or(usize::MAX),
            max_depth: usize::try_from(crate::quota::Quota::WkbDepth.rule().default)
                .unwrap_or(usize::MAX),
        }
    }
}

#[cfg(test)]
mod tests;
