use super::module::ModuleId;

/// Which section of the bar a module belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

/// Describes where modules are placed on the bar.
#[derive(Debug, Clone, Default)]
pub struct BarLayout {
    pub left: Vec<ModuleId>,
    pub center: Vec<ModuleId>,
    pub right: Vec<ModuleId>,
}

/// A computed region for a single module after layout.
#[derive(Debug, Clone)]
pub struct ModuleRegion {
    pub id: ModuleId,
    pub x: f64,
    pub width: f64,
}

impl BarLayout {
    /// Compute pixel regions for each module given the total bar width and a
    /// width-measurement callback.
    pub fn compute(
        &self,
        bar_width: f64,
        measure_width: &dyn Fn(&ModuleId) -> f64,
    ) -> Vec<ModuleRegion> {
        let mut regions = Vec::new();

        // Left-aligned
        let mut x = 0.0;
        for id in &self.left {
            let w = measure_width(id);
            regions.push(ModuleRegion {
                id: id.clone(),
                x,
                width: w,
            });
            x += w;
        }

        // Right-aligned (placed from the right edge)
        let mut right_x = bar_width;
        for id in self.right.iter().rev() {
            let w = measure_width(id);
            right_x -= w;
            regions.push(ModuleRegion {
                id: id.clone(),
                x: right_x,
                width: w,
            });
        }

        // Center-aligned
        let center_total: f64 = self.center.iter().map(|id| measure_width(id)).sum();
        let mut cx = (bar_width - center_total) / 2.0;
        for id in &self.center {
            let w = measure_width(id);
            regions.push(ModuleRegion {
                id: id.clone(),
                x: cx,
                width: w,
            });
            cx += w;
        }

        regions
    }
}
