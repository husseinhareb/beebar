use super::module::ModuleId;

/// Which section of the bar a module belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

/// Describes where modules are placed on the bar.
///
/// Each side is a list of groups, and each group is a list of module ids.
/// A "group" renders as one pill-shaped cluster sharing a background.
#[derive(Debug, Clone, Default)]
pub struct BarLayout {
    pub left: Vec<Vec<ModuleId>>,
    pub center: Vec<Vec<ModuleId>>,
    pub right: Vec<Vec<ModuleId>>,
}

/// A computed region for a single module after layout.
#[derive(Debug, Clone)]
pub struct ModuleRegion {
    pub id: ModuleId,
    pub x: f64,
    pub width: f64,
}

/// A computed region for a single pill group after layout.
#[derive(Debug, Clone)]
pub struct GroupRegion {
    pub x: f64,
    pub width: f64,
    pub modules: Vec<ModuleRegion>,
}

/// Spacing parameters used when computing the layout.
#[derive(Debug, Clone, Copy)]
pub struct LayoutSpacing {
    pub margin_left: f64,
    pub margin_right: f64,
    pub group_spacing: f64,
}

impl Default for LayoutSpacing {
    fn default() -> Self {
        Self {
            margin_left: 0.0,
            margin_right: 0.0,
            group_spacing: 0.0,
        }
    }
}

impl BarLayout {
    /// Compute pixel regions for each group (and the modules within them) given
    /// the total bar width and a width-measurement callback for each module.
    pub fn compute(
        &self,
        bar_width: f64,
        spacing: LayoutSpacing,
        measure_width: &dyn Fn(&ModuleId) -> f64,
    ) -> Vec<GroupRegion> {
        let mut groups = Vec::new();

        // ── Left-aligned groups: laid out left-to-right from `margin_left`.
        let mut x = spacing.margin_left;
        for (i, group) in self.left.iter().enumerate() {
            if i > 0 {
                x += spacing.group_spacing;
            }
            let region = build_group_region(x, group, measure_width);
            x += region.width;
            groups.push(region);
        }

        // ── Right-aligned groups: laid out right-to-left from `margin_right`.
        let mut right_x = bar_width - spacing.margin_right;
        for (i, group) in self.right.iter().enumerate().rev() {
            // The visual gap between groups happens on the right side of the
            // group we're about to place when there's already a group to its
            // right (i.e. anything except the truly rightmost iteration).
            if i < self.right.len().saturating_sub(1) {
                right_x -= spacing.group_spacing;
            }
            // Compute width by measuring at a temporary anchor, then shift.
            let probe = build_group_region(0.0, group, measure_width);
            right_x -= probe.width;
            groups.push(shift_group(probe, right_x));
        }

        // ── Center-aligned groups: centered as a block.
        let center_widths: Vec<f64> = self
            .center
            .iter()
            .map(|g| build_group_region(0.0, g, measure_width).width)
            .collect();
        let gaps_total = if self.center.is_empty() {
            0.0
        } else {
            spacing.group_spacing * (self.center.len() as f64 - 1.0)
        };
        let center_total: f64 = center_widths.iter().sum::<f64>() + gaps_total;
        let mut cx = (bar_width - center_total) / 2.0;
        for (i, group) in self.center.iter().enumerate() {
            if i > 0 {
                cx += spacing.group_spacing;
            }
            let region = build_group_region(cx, group, measure_width);
            cx += region.width;
            groups.push(region);
        }

        groups
    }

    /// Flatten all modules in a group list into a single vec, in order.
    pub fn flatten_modules(groups: &[GroupRegion]) -> Vec<ModuleRegion> {
        groups
            .iter()
            .flat_map(|g| g.modules.iter().cloned())
            .collect()
    }

    /// True when the bar has no groups defined at all.
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.center.is_empty() && self.right.is_empty()
    }
}

fn build_group_region(
    start_x: f64,
    group: &[ModuleId],
    measure_width: &dyn Fn(&ModuleId) -> f64,
) -> GroupRegion {
    let mut modules = Vec::with_capacity(group.len());
    let mut x = start_x;
    for id in group {
        let w = measure_width(id);
        modules.push(ModuleRegion {
            id: id.clone(),
            x,
            width: w,
        });
        x += w;
    }
    GroupRegion {
        x: start_x,
        width: x - start_x,
        modules,
    }
}

fn shift_group(mut group: GroupRegion, new_x: f64) -> GroupRegion {
    let delta = new_x - group.x;
    group.x = new_x;
    for m in &mut group.modules {
        m.x += delta;
    }
    group
}
