use x11rb::connection::Connection;
use x11rb::protocol::render::{ConnectionExt as RenderExt, PictType, Pictformat};
use x11rb::protocol::xproto::Visualid;

#[derive(Debug, Clone, Copy)]
pub struct AlphaVisualInfo {
    pub visual_id: Visualid,
    pub pict_format: Pictformat,
}

/// Discovers a 32-bit TrueColor visual that has a corresponding
/// X11 Render PictFormat with a non-zero (8-bit) alpha channel.
pub fn find_alpha_visual(conn: &impl Connection) -> Result<AlphaVisualInfo, String> {
    let formats = match conn.render_query_pict_formats() {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(e) => {
                return Err(format!(
                    "Failed to query RENDER picture formats reply: {}",
                    e
                ))
            }
        },
        Err(e) => {
            return Err(format!(
                "Failed to send RENDER query pict formats request: {}",
                e
            ))
        }
    };

    for screen in &formats.screens {
        for depth in &screen.depths {
            if depth.depth == 32 {
                for vis in &depth.visuals {
                    if let Some(fmt) = formats.formats.iter().find(|f| f.id == vis.format) {
                        if fmt.type_ == PictType::DIRECT && fmt.direct.alpha_mask == 0xff {
                            return Ok(AlphaVisualInfo {
                                visual_id: vis.visual,
                                pict_format: fmt.id,
                            });
                        }
                    }
                }
            }
        }
    }

    Err("No 32-bit visual with an alpha-capable Render format was found on this X11 display. Ensure a compositing manager (e.g. Picom) is active.".to_string())
}
