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
        let left_edge = x;

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
        let right_edge = right_x;

        // ── Center-aligned groups: centered as a block, then constrained to
        // the free lane between side groups. Without this, a long title can
        // draw under right-side status modules.
        let center_groups: Vec<GroupRegion> = self
            .center
            .iter()
            .map(|g| build_group_region(0.0, g, measure_width))
            .collect();

        let lane_start = if self.left.is_empty() {
            spacing.margin_left
        } else {
            left_edge + spacing.group_spacing
        };
        let lane_end = if self.right.is_empty() {
            bar_width - spacing.margin_right
        } else {
            right_edge - spacing.group_spacing
        };
        let lane_width = (lane_end - lane_start).max(0.0);

        let center_gap_count = self.center.len().saturating_sub(1) as f64;
        let center_gap = if center_gap_count <= 0.0 {
            0.0
        } else {
            spacing
                .group_spacing
                .min((lane_width / center_gap_count).max(0.0))
        };
        let gaps_total = center_gap * center_gap_count;
        let natural_modules_total: f64 = center_groups.iter().map(|group| group.width).sum();
        let max_modules_total = (lane_width - gaps_total).max(0.0);
        let fitted_groups =
            fit_groups_to_width(center_groups, natural_modules_total.min(max_modules_total));
        let center_total: f64 =
            fitted_groups.iter().map(|group| group.width).sum::<f64>() + gaps_total;

        let centered_x = (bar_width - center_total) / 2.0;
        let min_x = lane_start;
        let max_x = (lane_end - center_total).max(min_x);
        let mut cx = centered_x.clamp(min_x, max_x);
        for (i, group) in fitted_groups.into_iter().enumerate() {
            if i > 0 {
                cx += center_gap;
            }
            let region = shift_group(group, cx);
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

fn fit_groups_to_width(mut groups: Vec<GroupRegion>, target_width: f64) -> Vec<GroupRegion> {
    let natural_width: f64 = groups.iter().map(|group| group.width).sum();
    if natural_width <= target_width || natural_width <= 0.0 {
        return groups;
    }

    let scale = (target_width / natural_width).clamp(0.0, 1.0);
    for group in &mut groups {
        let mut x = group.x;
        for module in &mut group.modules {
            module.x = x;
            module.width *= scale;
            x += module.width;
        }
        group.width = x - group.x;
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::{BarLayout, GroupRegion, LayoutSpacing};

    fn width_for(id: &str) -> f64 {
        match id {
            "left" => 120.0,
            "center" => 600.0,
            "center-small" => 200.0,
            "right" => 220.0,
            "right-wide" => 350.0,
            other => panic!("unexpected module id: {other}"),
        }
    }

    fn group_with<'a>(groups: &'a [GroupRegion], id: &str) -> &'a GroupRegion {
        groups
            .iter()
            .find(|group| group.modules.iter().any(|module| module.id == id))
            .expect("group not found")
    }

    fn spacing() -> LayoutSpacing {
        LayoutSpacing {
            margin_left: 10.0,
            margin_right: 10.0,
            group_spacing: 10.0,
        }
    }

    #[test]
    fn center_group_is_shrunk_to_free_lane_between_sides() {
        let layout = BarLayout {
            left: vec![vec!["left".into()]],
            center: vec![vec!["center".into()]],
            right: vec![vec!["right".into()]],
        };

        let groups = layout.compute(800.0, spacing(), &|id| width_for(id));
        let center = group_with(&groups, "center");

        assert_eq!(center.x, 140.0);
        assert_eq!(center.width, 420.0);
        assert_eq!(center.x + center.width, 560.0);
    }

    #[test]
    fn center_group_stays_screen_centered_when_it_fits() {
        let layout = BarLayout {
            left: vec![vec!["left".into()]],
            center: vec![vec!["center-small".into()]],
            right: vec![vec!["right".into()]],
        };

        let groups = layout.compute(1000.0, spacing(), &|id| width_for(id));
        let center = group_with(&groups, "center-small");

        assert_eq!(center.x, 400.0);
        assert_eq!(center.width, 200.0);
    }

    #[test]
    fn center_group_shifts_away_from_wide_right_side() {
        let layout = BarLayout {
            left: vec![vec!["left".into()]],
            center: vec![vec!["center-small".into()]],
            right: vec![vec!["right-wide".into()]],
        };

        let groups = layout.compute(800.0, spacing(), &|id| width_for(id));
        let center = group_with(&groups, "center-small");

        assert_eq!(center.x, 230.0);
        assert_eq!(center.width, 200.0);
        assert_eq!(center.x + center.width, 430.0);
    }
}
