use niri_config::{Modifiers, Orientation};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputAxisPolicy {
    orientation: Orientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewWheelTarget {
    Column,
    Workspace,
}

impl InputAxisPolicy {
    pub const fn from_orientation(orientation: Orientation) -> Self {
        Self { orientation }
    }

    pub const fn orientation(self) -> Orientation {
        self.orientation
    }

    pub const fn is_vertical(self) -> bool {
        matches!(self.orientation, Orientation::Vertical)
    }

    pub fn gesture_prefers_view_offset(self, cumulative_x: f64, cumulative_y: f64) -> bool {
        if self.is_vertical() {
            cumulative_y.abs() > cumulative_x.abs()
        } else {
            cumulative_x.abs() > cumulative_y.abs()
        }
    }

    pub fn split_view_workspace_deltas(self, delta_x: f64, delta_y: f64) -> (f64, f64) {
        if self.is_vertical() {
            (delta_y, delta_x)
        } else {
            (delta_x, delta_y)
        }
    }

    pub fn overview_wheel_target(
        self,
        horizontal: bool,
        modifiers: Modifiers,
    ) -> Option<OverviewWheelTarget> {
        if horizontal {
            if modifiers.is_empty() {
                Some(if self.is_vertical() {
                    OverviewWheelTarget::Workspace
                } else {
                    OverviewWheelTarget::Column
                })
            } else {
                None
            }
        } else if modifiers.is_empty() {
            Some(if self.is_vertical() {
                OverviewWheelTarget::Column
            } else {
                OverviewWheelTarget::Workspace
            })
        } else if modifiers == Modifiers::SHIFT {
            Some(if self.is_vertical() {
                OverviewWheelTarget::Workspace
            } else {
                OverviewWheelTarget::Column
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_prefers_view_offset_respects_orientation() {
        let horizontal = InputAxisPolicy::from_orientation(Orientation::Horizontal);
        let vertical = InputAxisPolicy::from_orientation(Orientation::Vertical);

        assert!(horizontal.gesture_prefers_view_offset(20., 1.));
        assert!(!horizontal.gesture_prefers_view_offset(1., 20.));
        assert!(vertical.gesture_prefers_view_offset(1., 20.));
        assert!(!vertical.gesture_prefers_view_offset(20., 1.));
        assert!(!horizontal.gesture_prefers_view_offset(10., 10.));
        assert!(!vertical.gesture_prefers_view_offset(10., 10.));
    }

    #[test]
    fn split_view_workspace_deltas_respects_orientation() {
        let horizontal = InputAxisPolicy::from_orientation(Orientation::Horizontal);
        let vertical = InputAxisPolicy::from_orientation(Orientation::Vertical);

        assert_eq!(horizontal.split_view_workspace_deltas(3., -7.), (3., -7.));
        assert_eq!(vertical.split_view_workspace_deltas(3., -7.), (-7., 3.));
    }

    #[test]
    fn overview_wheel_target_respects_orientation_and_shift() {
        let horizontal = InputAxisPolicy::from_orientation(Orientation::Horizontal);
        let vertical = InputAxisPolicy::from_orientation(Orientation::Vertical);

        assert_eq!(
            horizontal.overview_wheel_target(true, Modifiers::empty()),
            Some(OverviewWheelTarget::Column)
        );
        assert_eq!(
            vertical.overview_wheel_target(true, Modifiers::empty()),
            Some(OverviewWheelTarget::Workspace)
        );

        assert_eq!(
            horizontal.overview_wheel_target(false, Modifiers::empty()),
            Some(OverviewWheelTarget::Workspace)
        );
        assert_eq!(
            horizontal.overview_wheel_target(false, Modifiers::SHIFT),
            Some(OverviewWheelTarget::Column)
        );

        assert_eq!(
            vertical.overview_wheel_target(false, Modifiers::empty()),
            Some(OverviewWheelTarget::Column)
        );
        assert_eq!(
            vertical.overview_wheel_target(false, Modifiers::SHIFT),
            Some(OverviewWheelTarget::Workspace)
        );

        assert_eq!(
            horizontal.overview_wheel_target(true, Modifiers::SHIFT),
            None
        );
    }
}
