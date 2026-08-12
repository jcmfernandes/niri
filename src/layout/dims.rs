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

// `smithay::output::Scale` does not implement `PartialEq`, so compare it by its
// fractional value, which is the only property anything reads it for.
impl PartialEq for Dims {
    fn eq(&self, other: &Self) -> bool {
        self.fractional_scale() == other.fractional_scale()
            && self.view_size == other.view_size
            && self.working_area == other.working_area
            && self.axis == other.axis
    }
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

    pub fn set_working_area(&mut self, working_area: Rectangle<f64, Logical>) {
        self.working_area = working_area;
    }

    pub fn axis(self) -> AxisMap {
        self.axis
    }

    pub fn orientation(self) -> Orientation {
        self.axis.main_axis()
    }

    pub fn view_size_main(self) -> f64 {
        self.axis.size_main(self.view_size)
    }

    pub fn view_size_cross(self) -> f64 {
        self.axis.size_cross(self.view_size)
    }

    pub fn working_area_main(self) -> f64 {
        self.axis.size_main(self.working_area.size)
    }

    pub fn working_area_cross(self) -> f64 {
        self.axis.size_cross(self.working_area.size)
    }
}

#[cfg(test)]
mod tests {
    use smithay::utils::Point;

    use super::*;

    #[test]
    fn main_and_cross_follow_orientation() {
        let size = Size::<f64, Logical>::from((1000., 600.));
        let area = Rectangle::new(Point::from((0., 0.)), size);
        let scale = smithay::output::Scale::Integer(1);

        let h = Dims::new(scale, size, area, Orientation::Horizontal);
        assert_eq!(h.view_size_main(), 1000.);
        assert_eq!(h.view_size_cross(), 600.);

        let v = Dims::new(scale, size, area, Orientation::Vertical);
        assert_eq!(v.view_size_main(), 600.);
        assert_eq!(v.view_size_cross(), 1000.);
    }

    #[test]
    fn working_area_main_and_cross_follow_orientation() {
        let size = Size::<f64, Logical>::from((1000., 600.));
        let area = Rectangle::new(Point::from((10., 20.)), Size::from((800., 400.)));
        let scale = smithay::output::Scale::Integer(1);

        let h = Dims::new(scale, size, area, Orientation::Horizontal);
        assert_eq!(h.working_area_main(), 800.);
        assert_eq!(h.working_area_cross(), 400.);

        let v = Dims::new(scale, size, area, Orientation::Vertical);
        assert_eq!(v.working_area_main(), 400.);
        assert_eq!(v.working_area_cross(), 800.);
    }
}
