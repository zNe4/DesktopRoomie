use x11rb::connection::Connection;
use x11rb::protocol::render::{ConnectionExt as RenderExt, PictType, Pictformat};
use x11rb::protocol::xproto::Visualid;

#[derive(Debug, Clone, Copy)]
pub struct AlphaVisualInfo {
    pub visual_id: Visualid,
    pub pict_format: Pictformat,
    pub alpha_mask: u16,
    pub red_mask: u16,
    pub green_mask: u16,
    pub blue_mask: u16,
    pub alpha_shift: u16,
    pub red_shift: u16,
    pub green_shift: u16,
    pub blue_shift: u16,
}

/// Discovers a 32-bit TrueColor visual that has a corresponding
/// X11 Render PictFormat with verified 8-bit alpha channel and standard
/// little-endian ARGB (BGRA memory layout: B@0, G@8, R@16, A@24).
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
                        let d = &fmt.direct;
                        // Verify 8-bit channels and standard 32-bit ARGB channel shifts
                        // (Blue shift 0, Green shift 8, Red shift 16, Alpha shift 24)
                        let is_standard_argb = fmt.type_ == PictType::DIRECT
                            && d.alpha_mask == 0xff
                            && d.red_mask == 0xff
                            && d.green_mask == 0xff
                            && d.blue_mask == 0xff
                            && d.blue_shift == 0
                            && d.green_shift == 8
                            && d.red_shift == 16
                            && d.alpha_shift == 24;

                        if is_standard_argb {
                            return Ok(AlphaVisualInfo {
                                visual_id: vis.visual,
                                pict_format: fmt.id,
                                alpha_mask: d.alpha_mask,
                                red_mask: d.red_mask,
                                green_mask: d.green_mask,
                                blue_mask: d.blue_mask,
                                alpha_shift: d.alpha_shift,
                                red_shift: d.red_shift,
                                green_shift: d.green_shift,
                                blue_shift: d.blue_shift,
                            });
                        }
                    }
                }
            }
        }
    }

    Err("No 32-bit visual with a standard 8-bit ARGB Render format (B=0, G=8, R=16, A=24 shifts) was found on this X11 display. Ensure a compositing manager (e.g. Picom) is active.".to_string())
}
