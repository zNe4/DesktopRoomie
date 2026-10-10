use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as ShapeExt, SK, SO};
use x11rb::protocol::xproto::{ClipOrdering, Rectangle, Window};

/// Applies an input shape mask to the window using the X11 Shape extension.
///
/// This shapes the `SK::INPUT` region to strictly cover the deliberate test shapes
/// (the circular body silhouette and the translucent test patch), while excluding
/// all transparent padding.
///
/// Clicks within the shaped region generate X11 pointer events on this window;
/// clicks in the transparent padding pass straight through to whatever window or
/// desktop surface is underneath.
pub fn apply_body_input_shape(
    conn: &impl Connection,
    window: Window,
    width: u16,
    height: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    restore_body_input_shape(conn, window, width, height).map(|_| ())
}

/// Returns the full checked enabling sequence for the application freshness gate.
pub fn restore_body_input_shape(
    conn: &impl Connection,
    window: Window,
    width: u16,
    height: u16,
) -> Result<u64, Box<dyn std::error::Error>> {
    set_input_with(window, silhouette_rectangles(width, height), |request| {
        request.send(conn)
    })
}

#[allow(dead_code)] // Stage D foundation; production transitions start in E.
pub fn empty_body_input_shape(
    conn: &impl Connection,
    window: Window,
) -> Result<u64, Box<dyn std::error::Error>> {
    set_input_with(window, vec![], |request| request.send(conn))
}

fn silhouette_rectangles(width: u16, height: u16) -> Vec<Rectangle> {
    let mut rects = Vec::new();

    for y in 0..height {
        let mut in_span = false;
        let mut span_start = 0u16;

        for x in 0..width {
            let is_hit = crate::geometry::is_in_interactive_silhouette(x as i16, y as i16);

            if is_hit && !in_span {
                in_span = true;
                span_start = x;
            } else if !is_hit && in_span {
                in_span = false;
                rects.push(Rectangle {
                    x: span_start as i16,
                    y: y as i16,
                    width: x - span_start,
                    height: 1,
                });
            }
        }

        if in_span {
            rects.push(Rectangle {
                x: span_start as i16,
                y: y as i16,
                width: width - span_start,
                height: 1,
            });
        }
    }

    rects
}

struct InputShapeRequest {
    operation: SO,
    kind: SK,
    ordering: ClipOrdering,
    window: Window,
    x: i16,
    y: i16,
    rectangles: Vec<Rectangle>,
}
impl InputShapeRequest {
    fn send(self, conn: &impl Connection) -> Result<u64, Box<dyn std::error::Error>> {
        let cookie = conn.shape_rectangles(
            self.operation,
            self.kind,
            self.ordering,
            self.window,
            self.x,
            self.y,
            &self.rectangles,
        )?;
        let sequence = cookie.sequence_number();
        cookie.check()?;
        Ok(sequence)
    }
}
fn set_input_with(
    window: Window,
    rectangles: Vec<Rectangle>,
    checked: impl FnOnce(InputShapeRequest) -> Result<u64, Box<dyn std::error::Error>>,
) -> Result<u64, Box<dyn std::error::Error>> {
    checked(InputShapeRequest {
        operation: SO::SET,
        kind: SK::INPUT,
        ordering: ClipOrdering::UNSORTED,
        window,
        x: 0,
        y: 0,
        rectangles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restored_shape_matches_every_original_silhouette_pixel_including_patch_and_padding() {
        let rects = silhouette_rectangles(160, 160);
        for y in -1..=160i16 {
            for x in -1..=160i16 {
                let hits = rects
                    .iter()
                    .filter(|r| {
                        x >= r.x
                            && y >= r.y
                            && i32::from(x) < i32::from(r.x) + i32::from(r.width)
                            && i32::from(y) < i32::from(r.y) + i32::from(r.height)
                    })
                    .count();
                assert_eq!(
                    hits,
                    usize::from(crate::geometry::is_in_interactive_silhouette(x, y)),
                    "{x},{y}"
                );
            }
        }
    }
    #[test]
    fn empty_and_restored_requests_are_checked_exact_input_sets() {
        for rectangles in [vec![], silhouette_rectangles(160, 160)] {
            for fail in [false, true] {
                let result = set_input_with(200, rectangles.clone(), |request| {
                    assert_eq!(request.operation, SO::SET);
                    assert_eq!(request.kind, SK::INPUT);
                    assert_eq!(request.ordering, ClipOrdering::UNSORTED);
                    assert_eq!((request.window, request.x, request.y), (200, 0, 0));
                    let fields = |r: &Rectangle| (r.x, r.y, r.width, r.height);
                    assert_eq!(
                        request.rectangles.iter().map(fields).collect::<Vec<_>>(),
                        rectangles.iter().map(fields).collect::<Vec<_>>()
                    );
                    if fail {
                        Err("SHAPE check failed".into())
                    } else {
                        Ok(70000)
                    }
                });
                if fail {
                    assert!(result.is_err());
                } else {
                    assert_eq!(result.unwrap(), 70000);
                }
            }
        }
    }
}
