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
    let mut rects = Vec::new();

    for y in 0..height {
        let mut in_span = false;
        let mut span_start = 0u16;

        for x in 0..width {
            let dx = (x as f32) - 80.0;
            let dy = (y as f32) - 80.0;
            let dist = (dx * dx + dy * dy).sqrt();

            let is_body = dist <= 45.0;
            let is_patch = (15..=65).contains(&x) && (15..=65).contains(&y);
            let is_hit = is_body || is_patch;

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

    conn.shape_rectangles(
        SO::SET,
        SK::INPUT,
        ClipOrdering::UNSORTED,
        window,
        0,
        0,
        &rects,
    )?
    .check()?;

    Ok(())
}
