use niri_config::Orientation;
use smithay::utils::{Logical, Rectangle, Size};

use super::axis::AxisMap;

/// View geometry bundled with the axis that interprets it.
#[derive(Debug, Clone, Copy)]
pub struct Dims {
    scale: smithay::output::Scale,
    view_size: Size<f64, Logical>,
    working_area: Rectangle<f64, Logical>,
    axis: AxisMap,
}

impl Dims {
    pub fn new(
        scale: smithay::output::Scale,
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        orientation: Orientation,
    ) -> Self {
        Self {
            scale,
            view_size,
            working_area,
            axis: AxisMap::new(orientation),
        }
    }

    pub fn scale(self) -> smithay::output::Scale {
        self.scale
    }

    pub fn fractional_scale(self) -> f64 {
        self.scale.fractional_scale()
    }

    pub fn view_size(self) -> Size<f64, Logical> {
        self.view_size
    }

    pub fn working_area(self) -> Rectangle<f64, Logical> {
        self.working_area
    }

    pub fn axis(self) -> AxisMap {
        self.axis
    }

    pub fn orientation(self) -> Orientation {
        self.axis.main_axis()
    }
}
