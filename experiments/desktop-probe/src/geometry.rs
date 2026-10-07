#![allow(dead_code)]

use std::fmt;

/// A signed 2D coordinate representing a position in root or client coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// 2D dimensions with non-negative magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// A 2D rectangle in signed screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Checked right edge x-coordinate (`x + width`) using wide arithmetic.
    pub fn checked_right(&self) -> Option<i32> {
        let r = (self.x as i64).checked_add(self.width as i64)?;
        i32::try_from(r).ok()
    }

    /// Checked bottom edge y-coordinate (`y + height`) using wide arithmetic.
    pub fn checked_bottom(&self) -> Option<i32> {
        let b = (self.y as i64).checked_add(self.height as i64)?;
        i32::try_from(b).ok()
    }

    pub fn right(&self) -> Result<i32, PlacementError> {
        self.checked_right()
            .ok_or(PlacementError::CoordinateOverflow)
    }

    pub fn bottom(&self) -> Result<i32, PlacementError> {
        self.checked_bottom()
            .ok_or(PlacementError::CoordinateOverflow)
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Rect(x={}, y={}, w={}, h={})",
            self.x, self.y, self.width, self.height
        )
    }
}

/// Vector displacement between a pointer root position and a window origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GrabOffset {
    pub dx: i32,
    pub dy: i32,
}

impl GrabOffset {
    pub const fn new(dx: i32, dy: i32) -> Self {
        Self { dx, dy }
    }

    pub fn from_points(pointer: Point, origin: Point) -> Result<Self, PlacementError> {
        calculate_grab_offset(pointer, origin)
    }
}

impl fmt::Display for GrabOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GrabOffset(dx={}, dy={})", self.dx, self.dy)
    }
}

/// An inclusive range of valid window origins within an allowable desktop area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidOriginBounds {
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
}

impl ValidOriginBounds {
    pub const fn new(min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> Self {
        Self {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    pub fn try_new(min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> Result<Self, PlacementError> {
        if min_x > max_x || min_y > max_y {
            return Err(PlacementError::CoordinateOverflow);
        }
        Ok(Self {
            min_x,
            max_x,
            min_y,
            max_y,
        })
    }

    /// Clamps requested root origin to ensure the window remains entirely within valid bounds.
    pub fn clamp(&self, target: Point) -> Point {
        Point {
            x: target.x.clamp(self.min_x, self.max_x),
            y: target.y.clamp(self.min_y, self.max_y),
        }
    }

    /// Checks if the target origin is within valid bounds.
    pub fn contains(&self, target: Point) -> bool {
        target.x >= self.min_x
            && target.x <= self.max_x
            && target.y >= self.min_y
            && target.y <= self.max_y
    }
}

impl fmt::Display for ValidOriginBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "x={}..={}, y={}..={}",
            self.min_x, self.max_x, self.min_y, self.max_y
        )
    }
}

/// Errors when validating placement against a usable desktop area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    EmptyUsableArea,
    WindowExceedsBounds {
        usable_width: u32,
        usable_height: u32,
        required_width: u32,
        required_height: u32,
    },
    CoordinateOverflow,
}

impl fmt::Display for PlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUsableArea => write!(f, "Usable area is empty (width or height is 0)"),
            Self::WindowExceedsBounds {
                usable_width,
                usable_height,
                required_width,
                required_height,
            } => write!(
                f,
                "Window size ({}x{}) exceeds usable area bounds ({}x{})",
                required_width, required_height, usable_width, usable_height
            ),
            Self::CoordinateOverflow => {
                write!(f, "Coordinate arithmetic overflowed representable bounds")
            }
        }
    }
}

impl std::error::Error for PlacementError {}

/// Menu placement is independent of the body's fixed canvas bounds.
pub fn place_menu(area: Rect, size: Size, anchor: Point, gap: u32) -> Result<Rect, PlacementError> {
    if size.width == 0 || size.height == 0 {
        return Err(PlacementError::EmptyUsableArea);
    }
    let bounds = compute_valid_origin_bounds(area, size)?;
    let axis = |anchor: i32, length: u32, min: i32, max: i32| {
        let preferred = i64::from(anchor) + i64::from(gap);
        let flipped = i64::from(anchor) - i64::from(gap) - i64::from(length);
        if (i64::from(min)..=i64::from(max)).contains(&preferred) {
            preferred
        } else if (i64::from(min)..=i64::from(max)).contains(&flipped) {
            flipped
        } else {
            preferred.clamp(i64::from(min), i64::from(max))
        }
    };
    let x = axis(anchor.x, size.width, bounds.min_x, bounds.max_x);
    let y = axis(anchor.y, size.height, bounds.min_y, bounds.max_y);
    // Validate wire dimensions and origins here so unsupported placement is recoverable.
    i16::try_from(x).map_err(|_| PlacementError::CoordinateOverflow)?;
    i16::try_from(y).map_err(|_| PlacementError::CoordinateOverflow)?;
    u16::try_from(size.width).map_err(|_| PlacementError::CoordinateOverflow)?;
    u16::try_from(size.height).map_err(|_| PlacementError::CoordinateOverflow)?;
    Ok(Rect::new(x as i32, y as i32, size.width, size.height))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHit {
    Row(usize),
    Background,
    Outside,
}

/// The single-pixel outer border and separator are intentionally inert.
#[derive(Debug, Clone, Copy)]
pub struct MenuLayout {
    pub size: Size,
    pub rows: [Rect; 2],
}

impl MenuLayout {
    pub fn new(width: u32, row_height: u32) -> Result<Self, PlacementError> {
        if width < 3 || row_height < 3 {
            return Err(PlacementError::EmptyUsableArea);
        }
        let height = row_height
            .checked_mul(2)
            .and_then(|h| h.checked_add(3))
            .ok_or(PlacementError::CoordinateOverflow)?;
        // Drawing coordinates as well as window dimensions must be representable.
        i16::try_from(width).map_err(|_| PlacementError::CoordinateOverflow)?;
        i16::try_from(height).map_err(|_| PlacementError::CoordinateOverflow)?;
        Ok(Self {
            size: Size::new(width, height),
            rows: [
                Rect::new(1, 1, width - 2, row_height),
                Rect::new(1, (row_height + 2) as i32, width - 2, row_height),
            ],
        })
    }

    pub fn hit(&self, local: Point) -> MenuHit {
        let contains = |rect: Rect| {
            i64::from(local.x) >= i64::from(rect.x)
                && i64::from(local.y) >= i64::from(rect.y)
                && i64::from(local.x) < i64::from(rect.x) + i64::from(rect.width)
                && i64::from(local.y) < i64::from(rect.y) + i64::from(rect.height)
        };
        if !contains(Rect::new(0, 0, self.size.width, self.size.height)) {
            return MenuHit::Outside;
        }
        self.rows
            .iter()
            .position(|row| contains(*row))
            .map_or(MenuHit::Background, MenuHit::Row)
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn center_edges_and_four_corners_flip_independently() {
        let area = Rect::new(10, 48, 400, 300);
        let size = Size::new(120, 60);
        for (anchor, expected) in [
            ((200, 150), (204, 154)),
            ((10, 150), (14, 154)),
            ((409, 150), (285, 154)),
            ((200, 48), (204, 52)),
            ((200, 347), (204, 283)),
            ((10, 48), (14, 52)),
            ((409, 48), (285, 52)),
            ((10, 347), (14, 283)),
            ((409, 347), (285, 283)),
        ] {
            let rect = place_menu(area, size, Point::new(anchor.0, anchor.1), 4).unwrap();
            assert_eq!(rect.origin(), Point::new(expected.0, expected.1));
            assert!(rect.x >= area.x && rect.y >= area.y);
            assert!(rect.right().unwrap() <= area.right().unwrap());
            assert!(rect.bottom().unwrap() <= area.bottom().unwrap());
        }
    }

    #[test]
    fn negative_origin_exact_fit_and_final_clamp() {
        let area = Rect::new(-400, -300, 400, 300);
        let size = Size::new(120, 60);
        assert_eq!(
            place_menu(area, size, Point::new(-1, -1), 4)
                .unwrap()
                .origin(),
            Point::new(-125, -65)
        );
        assert_eq!(
            place_menu(area, area.size(), Point::new(-200, -150), 4).unwrap(),
            area
        );
        assert_eq!(
            place_menu(area, size, Point::new(i32::MAX, i32::MIN), 4)
                .unwrap()
                .origin(),
            Point::new(-120, -300)
        );
    }

    #[test]
    fn invalid_popup_area_overflow_and_wire_coordinates_are_rejected() {
        let size = Size::new(120, 60);
        for area in [
            Rect::new(0, 0, 119, 60),
            Rect::new(0, 0, 120, 59),
            Rect::new(0, 0, 0, 60),
            Rect::new(i32::MAX, 0, 120, 60),
            Rect::new(40000, 0, 120, 60),
        ] {
            assert!(place_menu(area, size, Point::new(0, 0), 4).is_err());
        }
        assert!(place_menu(
            Rect::new(0, 0, 100000, 100000),
            Size::new(70000, 60),
            Point::new(0, 0),
            4
        )
        .is_err());
        assert!(place_menu(
            Rect::new(0, 0, 120, 60),
            Size::new(0, 60),
            Point::new(0, 0),
            4
        )
        .is_err());
        assert!(MenuLayout::new(120, u32::MAX).is_err());
    }

    #[test]
    fn rows_border_separator_and_exclusive_outer_edges() {
        let layout = MenuLayout::new(120, 28).unwrap();
        for (point, hit) in [
            ((1, 1), MenuHit::Row(0)),
            ((118, 28), MenuHit::Row(0)),
            ((1, 30), MenuHit::Row(1)),
            ((118, 57), MenuHit::Row(1)),
            ((0, 10), MenuHit::Background),
            ((119, 10), MenuHit::Background),
            ((12, 29), MenuHit::Background),
            ((12, 58), MenuHit::Background),
            ((-1, 10), MenuHit::Outside),
            ((120, 10), MenuHit::Outside),
            ((12, 59), MenuHit::Outside),
        ] {
            assert_eq!(layout.hit(Point::new(point.0, point.1)), hit);
        }
    }
}

/// Computes the valid window origin range such that the entire body fits inside `usable`.
/// Uses wider intermediate arithmetic (i64) and checked conversions to prevent wrapping/overflow.
pub fn compute_valid_origin_bounds(
    usable: Rect,
    body: Size,
) -> Result<ValidOriginBounds, PlacementError> {
    if usable.width == 0 || usable.height == 0 {
        return Err(PlacementError::EmptyUsableArea);
    }

    if usable.width < body.width || usable.height < body.height {
        return Err(PlacementError::WindowExceedsBounds {
            usable_width: usable.width,
            usable_height: usable.height,
            required_width: body.width,
            required_height: body.height,
        });
    }

    let span_x = (usable.width - body.width) as i64;
    let span_y = (usable.height - body.height) as i64;

    let min_x = usable.x;
    let max_x_i64 = (usable.x as i64)
        .checked_add(span_x)
        .ok_or(PlacementError::CoordinateOverflow)?;
    let max_x = i32::try_from(max_x_i64).map_err(|_| PlacementError::CoordinateOverflow)?;

    let min_y = usable.y;
    let max_y_i64 = (usable.y as i64)
        .checked_add(span_y)
        .ok_or(PlacementError::CoordinateOverflow)?;
    let max_y = i32::try_from(max_y_i64).map_err(|_| PlacementError::CoordinateOverflow)?;

    if max_x < min_x || max_y < min_y {
        return Err(PlacementError::CoordinateOverflow);
    }

    Ok(ValidOriginBounds::new(min_x, max_x, min_y, max_y))
}

/// Calculates the centered position of `body` inside `usable`.
/// Uses wider intermediate arithmetic (i64) and checked conversions to prevent wrapping/overflow.
pub fn calculate_centered_origin(usable: Rect, body: Size) -> Result<Point, PlacementError> {
    compute_valid_origin_bounds(usable, body)?;
    let span_x = (usable.width - body.width) as i64;
    let span_y = (usable.height - body.height) as i64;

    let x_i64 = (usable.x as i64)
        .checked_add(span_x / 2)
        .ok_or(PlacementError::CoordinateOverflow)?;
    let y_i64 = (usable.y as i64)
        .checked_add(span_y / 2)
        .ok_or(PlacementError::CoordinateOverflow)?;

    let x = i32::try_from(x_i64).map_err(|_| PlacementError::CoordinateOverflow)?;
    let y = i32::try_from(y_i64).map_err(|_| PlacementError::CoordinateOverflow)?;

    Ok(Point::new(x, y))
}

/// Checked grab offset from pointer and origin coordinates.
pub fn checked_grab_offset(pointer: Point, origin: Point) -> Option<GrabOffset> {
    let dx_i64 = (pointer.x as i64) - (origin.x as i64);
    let dy_i64 = (pointer.y as i64) - (origin.y as i64);
    let dx = i32::try_from(dx_i64).ok()?;
    let dy = i32::try_from(dy_i64).ok()?;
    Some(GrabOffset::new(dx, dy))
}

/// Derives the grab offset from a pointer press and window origin: `grab_offset = pointer - origin`.
pub fn calculate_grab_offset(pointer: Point, origin: Point) -> Result<GrabOffset, PlacementError> {
    checked_grab_offset(pointer, origin).ok_or(PlacementError::CoordinateOverflow)
}

/// Checked target origin from pointer root position and grab offset.
pub fn checked_target_origin(pointer: Point, offset: GrabOffset) -> Option<Point> {
    let x_i64 = (pointer.x as i64) - (offset.dx as i64);
    let y_i64 = (pointer.y as i64) - (offset.dy as i64);
    let x = i32::try_from(x_i64).ok()?;
    let y = i32::try_from(y_i64).ok()?;
    Some(Point::new(x, y))
}

/// Calculates the requested origin from pointer root position and grab offset: `requested_origin = pointer - grab_offset`.
pub fn calculate_target_origin(
    pointer: Point,
    offset: GrabOffset,
) -> Result<Point, PlacementError> {
    checked_target_origin(pointer, offset).ok_or(PlacementError::CoordinateOverflow)
}

pub const DRAG_THRESHOLD_SQUARED: i128 = 16;

/// Returns true if the Euclidean distance between two points reaches or exceeds 4 pixels
/// (squared distance dx*dx + dy*dy >= 16) using 128-bit signed arithmetic to prevent
/// overflow when squaring differences between extreme i32 coordinates.
pub fn exceeds_drag_threshold(p1: Point, p2: Point) -> bool {
    let dx = (p2.x as i128) - (p1.x as i128);
    let dy = (p2.y as i128) - (p1.y as i128);
    dx * dx + dy * dy >= DRAG_THRESHOLD_SQUARED
}

/// Returns true if the local coordinates (relative to window top-left 0..160)
/// fall inside the interactive shape (circular body silhouette or translucent test patch).
pub fn is_in_interactive_silhouette(x: i16, y: i16) -> bool {
    if !(0..160).contains(&x) || !(0..160).contains(&y) {
        return false;
    }
    let dx = (x as f32) - 80.0;
    let dy = (y as f32) - 80.0;
    let dist = (dx * dx + dy * dy).sqrt();
    let is_body = dist <= 45.0;
    let is_patch = (15..=65).contains(&x) && (15..=65).contains(&y);
    is_body || is_patch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_four_edges() {
        let usable = Rect::new(100, 200, 800, 600);
        let body = Size::new(160, 160);
        let bounds = compute_valid_origin_bounds(usable, body).unwrap();

        assert_eq!(bounds.min_x, 100);
        assert_eq!(bounds.max_x, 100 + (800 - 160)); // 740
        assert_eq!(bounds.min_y, 200);
        assert_eq!(bounds.max_y, 200 + (600 - 160)); // 640

        // Beyond left edge
        assert_eq!(bounds.clamp(Point::new(50, 300)), Point::new(100, 300));
        // Beyond right edge
        assert_eq!(bounds.clamp(Point::new(800, 300)), Point::new(740, 300));
        // Beyond top edge
        assert_eq!(bounds.clamp(Point::new(300, 150)), Point::new(300, 200));
        // Beyond bottom edge
        assert_eq!(bounds.clamp(Point::new(300, 700)), Point::new(300, 640));
    }

    #[test]
    fn test_all_corners() {
        let usable = Rect::new(10, 48, 1910, 1032);
        let body = Size::new(160, 160);
        let bounds = compute_valid_origin_bounds(usable, body).unwrap();

        // Top-left
        assert_eq!(bounds.clamp(Point::new(-100, -50)), Point::new(10, 48));
        // Top-right
        assert_eq!(bounds.clamp(Point::new(3000, -50)), Point::new(1760, 48));
        // Bottom-left
        assert_eq!(bounds.clamp(Point::new(-100, 2000)), Point::new(10, 920));
        // Bottom-right
        assert_eq!(bounds.clamp(Point::new(3000, 2000)), Point::new(1760, 920));
    }

    #[test]
    fn test_negative_origin() {
        // Multi-head monitor setup with negative display origin (e.g. secondary monitor to the left)
        let usable = Rect::new(-1920, -100, 1920, 1080);
        let body = Size::new(160, 160);
        let bounds = compute_valid_origin_bounds(usable, body).unwrap();

        assert_eq!(bounds.min_x, -1920);
        assert_eq!(bounds.max_x, -1920 + 1920 - 160); // -160
        assert_eq!(bounds.min_y, -100);
        assert_eq!(bounds.max_y, -100 + 1080 - 160); // 820

        // Clamping inside negative origin
        assert_eq!(
            bounds.clamp(Point::new(-2000, -200)),
            Point::new(-1920, -100)
        );
        assert_eq!(bounds.clamp(Point::new(0, 900)), Point::new(-160, 820));

        // Centered origin calculation
        let centered = calculate_centered_origin(usable, body).unwrap();
        assert_eq!(centered.x, -1920 + (1920 - 160) / 2); // -1040
        assert_eq!(centered.y, -100 + (1080 - 160) / 2); // 360
        assert!(bounds.contains(centered));
    }

    #[test]
    fn test_panel_reduced_region() {
        // Simulated Openbox desktop workarea with panel reservation:
        // Full screen is 1920x1080, top panel occupies 48px, side dock occupies 10px
        let usable = Rect::new(10, 48, 1910, 1032);
        let body = Size::new(160, 160);

        let bounds = compute_valid_origin_bounds(usable, body).unwrap();
        assert_eq!(bounds.min_x, 10);
        assert_eq!(bounds.max_x, 10 + 1910 - 160); // 1760
        assert_eq!(bounds.min_y, 48);
        assert_eq!(bounds.max_y, 48 + 1032 - 160); // 920

        let centered = calculate_centered_origin(usable, body).unwrap();
        assert_eq!(centered.x, 10 + (1910 - 160) / 2); // 885
        assert_eq!(centered.y, 48 + (1032 - 160) / 2); // 484
        assert!(bounds.contains(centered));
    }

    #[test]
    fn test_body_exactly_filling_area() {
        let usable = Rect::new(50, 100, 160, 160);
        let body = Size::new(160, 160);

        let bounds = compute_valid_origin_bounds(usable, body).unwrap();
        assert_eq!(bounds.min_x, 50);
        assert_eq!(bounds.max_x, 50);
        assert_eq!(bounds.min_y, 100);
        assert_eq!(bounds.max_y, 100);

        // Any requested point clamps to the single valid origin
        assert_eq!(bounds.clamp(Point::new(0, 0)), Point::new(50, 100));
        assert_eq!(bounds.clamp(Point::new(200, 300)), Point::new(50, 100));

        let centered = calculate_centered_origin(usable, body).unwrap();
        assert_eq!(centered, Point::new(50, 100));
    }

    #[test]
    fn test_body_too_large_and_empty_region() {
        let body = Size::new(160, 160);

        // Empty area
        let empty_w = Rect::new(0, 0, 0, 100);
        assert_eq!(
            compute_valid_origin_bounds(empty_w, body),
            Err(PlacementError::EmptyUsableArea)
        );
        let empty_h = Rect::new(0, 0, 100, 0);
        assert_eq!(
            compute_valid_origin_bounds(empty_h, body),
            Err(PlacementError::EmptyUsableArea)
        );

        // Body too wide
        let too_narrow = Rect::new(0, 0, 150, 200);
        assert_eq!(
            compute_valid_origin_bounds(too_narrow, body),
            Err(PlacementError::WindowExceedsBounds {
                usable_width: 150,
                usable_height: 200,
                required_width: 160,
                required_height: 160,
            })
        );

        // Body too tall
        let too_short = Rect::new(0, 0, 200, 150);
        assert_eq!(
            compute_valid_origin_bounds(too_short, body),
            Err(PlacementError::WindowExceedsBounds {
                usable_width: 200,
                usable_height: 150,
                required_width: 160,
                required_height: 160,
            })
        );
    }

    #[test]
    fn test_grab_offset_and_target_calculation() {
        let actual_origin = Point::new(885, 484);
        let pointer_press = Point::new(955, 539);

        // Offset = (955 - 885, 539 - 484) = (70, 55)
        let offset = calculate_grab_offset(pointer_press, actual_origin).unwrap();
        assert_eq!(offset, GrabOffset::new(70, 55));

        // When pointer moves to (1200, 700), requested origin = (1200 - 70, 700 - 55) = (1130, 645)
        let pointer_move = Point::new(1200, 700);
        let target = calculate_target_origin(pointer_move, offset).unwrap();
        assert_eq!(target, Point::new(1130, 645));

        // Clamping the requested origin
        let bounds = ValidOriginBounds::new(10, 1760, 48, 920);
        let clamped = bounds.clamp(target);
        assert_eq!(clamped, Point::new(1130, 645)); // Inside bounds

        // Extreme pointer move clamped
        let extreme_pointer = Point::new(2500, 1500);
        let extreme_target = calculate_target_origin(extreme_pointer, offset).unwrap();
        let extreme_clamped = bounds.clamp(extreme_target);
        assert_eq!(extreme_clamped, Point::new(1760, 920));
    }

    #[test]
    fn test_rect_methods_and_display() {
        let r = Rect::new(10, 20, 100, 200);
        assert_eq!(r.origin(), Point::new(10, 20));
        assert_eq!(r.size(), Size::new(100, 200));
        assert_eq!(r.right(), Ok(110));
        assert_eq!(r.bottom(), Ok(220));
        assert_eq!(r.checked_right(), Some(110));
        assert_eq!(r.checked_bottom(), Some(220));
        assert_eq!(format!("{}", r), "Rect(x=10, y=20, w=100, h=200)");
        assert_eq!(format!("{}", Point::new(5, 15)), "(5, 15)");
        assert_eq!(format!("{}", Size::new(160, 160)), "160x160");
        assert_eq!(
            format!("{}", GrabOffset::new(7, 8)),
            "GrabOffset(dx=7, dy=8)"
        );
    }

    #[test]
    fn test_overflow_usable_dimensions_and_inverted_bounds() {
        let body = Size::new(160, 160);

        // Usable width u32::MAX with body width 160 must not wrap into a negative max_x
        let huge_w = Rect::new(0, 0, u32::MAX, 1000);
        assert_eq!(
            compute_valid_origin_bounds(huge_w, body),
            Err(PlacementError::CoordinateOverflow)
        );

        // Usable height u32::MAX
        let huge_h = Rect::new(0, 0, 1000, u32::MAX);
        assert_eq!(
            compute_valid_origin_bounds(huge_h, body),
            Err(PlacementError::CoordinateOverflow)
        );

        // Centering calculation must also reject unrepresentable dimensions
        assert_eq!(
            calculate_centered_origin(huge_w, body),
            Err(PlacementError::CoordinateOverflow)
        );

        // Inverted bounds cannot be constructed via try_new
        assert_eq!(
            ValidOriginBounds::try_new(500, 100, 0, 100),
            Err(PlacementError::CoordinateOverflow)
        );
    }

    #[test]
    fn test_overflow_coordinates_near_limits() {
        let body = Size::new(160, 160);

        // Usable origin near i32::MAX where span causes overflow
        let near_max_x = Rect::new(i32::MAX - 20, 0, 200, 200);
        assert_eq!(
            compute_valid_origin_bounds(near_max_x, body),
            Err(PlacementError::CoordinateOverflow)
        );

        let near_max_y = Rect::new(0, i32::MAX - 20, 200, 200);
        assert_eq!(
            compute_valid_origin_bounds(near_max_y, body),
            Err(PlacementError::CoordinateOverflow)
        );

        // Rect edges near i32::MAX
        let r_overflow_x = Rect::new(i32::MAX - 5, 0, 10, 10);
        assert_eq!(
            r_overflow_x.right(),
            Err(PlacementError::CoordinateOverflow)
        );
        assert_eq!(r_overflow_x.checked_right(), None);

        let r_overflow_y = Rect::new(0, i32::MAX - 5, 10, 10);
        assert_eq!(
            r_overflow_y.bottom(),
            Err(PlacementError::CoordinateOverflow)
        );
        assert_eq!(r_overflow_y.checked_bottom(), None);

        // Grab offset overflow: distance from i32::MAX to i32::MIN exceeds i32
        assert_eq!(
            calculate_grab_offset(Point::new(i32::MAX, 0), Point::new(i32::MIN, 0)),
            Err(PlacementError::CoordinateOverflow)
        );
        assert_eq!(
            checked_grab_offset(Point::new(i32::MAX, 0), Point::new(i32::MIN, 0)),
            None
        );

        // Target origin overflow: subtracting positive offset from i32::MIN or negative from i32::MAX
        assert_eq!(
            calculate_target_origin(Point::new(i32::MIN, 0), GrabOffset::new(10, 0)),
            Err(PlacementError::CoordinateOverflow)
        );
        assert_eq!(
            calculate_target_origin(Point::new(i32::MAX, 0), GrabOffset::new(-10, 0)),
            Err(PlacementError::CoordinateOverflow)
        );
    }

    #[test]
    fn test_drag_threshold() {
        let p0 = Point::new(100, 100);

        // Identical point: 0px -> false
        assert!(!exceeds_drag_threshold(p0, p0));

        // 1px, 2px, 3px movement: squared distance < 16 -> false
        assert!(!exceeds_drag_threshold(p0, Point::new(101, 100)));
        assert!(!exceeds_drag_threshold(p0, Point::new(102, 102))); // 4 + 4 = 8 < 16
        assert!(!exceeds_drag_threshold(p0, Point::new(103, 102))); // 9 + 4 = 13 < 16
        assert!(!exceeds_drag_threshold(p0, Point::new(100, 103))); // 0 + 9 = 9 < 16

        // Exact threshold: 4px horizontal or vertical -> 16 >= 16 -> true
        assert!(exceeds_drag_threshold(p0, Point::new(104, 100))); // 16 >= 16
        assert!(exceeds_drag_threshold(p0, Point::new(96, 100))); // 16 >= 16
        assert!(exceeds_drag_threshold(p0, Point::new(100, 104))); // 16 >= 16
        assert!(exceeds_drag_threshold(p0, Point::new(100, 96))); // 16 >= 16

        // Diagonal threshold: dx=3, dy=3 -> 9 + 9 = 18 >= 16 -> true
        assert!(exceeds_drag_threshold(p0, Point::new(103, 103)));
        assert!(exceeds_drag_threshold(p0, Point::new(97, 97)));

        // Large distance
        assert!(exceeds_drag_threshold(p0, Point::new(200, 300)));

        // Extreme coordinate ranges (i32::MIN to i32::MAX) must not overflow i128
        let p_min = Point::new(i32::MIN, i32::MIN);
        let p_max = Point::new(i32::MAX, i32::MAX);
        assert!(exceeds_drag_threshold(p_min, p_max));
        assert!(exceeds_drag_threshold(p_max, p_min));
        assert!(!exceeds_drag_threshold(p_min, p_min));
        assert!(!exceeds_drag_threshold(p_max, p_max));
    }

    #[test]
    fn test_interactive_silhouette_hit_test() {
        // Circle center (80, 80) -> true
        assert!(is_in_interactive_silhouette(80, 80));

        // Inside circular body (radius 45)
        assert!(is_in_interactive_silhouette(80, 35)); // Top edge of circle
        assert!(is_in_interactive_silhouette(80, 125)); // Bottom edge of circle
        assert!(is_in_interactive_silhouette(35, 80)); // Left edge of circle
        assert!(is_in_interactive_silhouette(125, 80)); // Right edge of circle

        // Outside circular body (e.g. radius 50)
        assert!(!is_in_interactive_silhouette(80, 29));
        assert!(!is_in_interactive_silhouette(80, 131));

        // Translucent test patch: x in 15..=65, y in 15..=65 -> true
        assert!(is_in_interactive_silhouette(15, 15));
        assert!(is_in_interactive_silhouette(65, 65));
        assert!(is_in_interactive_silhouette(40, 40));

        // Transparent padding outside both circle and patch
        assert!(!is_in_interactive_silhouette(5, 5));
        assert!(!is_in_interactive_silhouette(155, 155));
        assert!(!is_in_interactive_silhouette(150, 10));
        assert!(!is_in_interactive_silhouette(10, 150));

        // Out of window bounds
        assert!(!is_in_interactive_silhouette(-1, 80));
        assert!(!is_in_interactive_silhouette(80, -1));
        assert!(!is_in_interactive_silhouette(160, 80));
        assert!(!is_in_interactive_silhouette(80, 160));
    }
}
