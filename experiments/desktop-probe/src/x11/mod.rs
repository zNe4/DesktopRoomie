pub mod control;
pub mod desktop;
pub(super) mod events;
pub mod menu;
pub mod monitors;
pub mod pointer;
pub mod render;
pub mod resource;
pub mod shape;
pub mod state;
pub mod visibility;
pub mod visual;
pub mod window;

use crate::presentation::ClientExtent;

fn native_dimensions(extent: ClientExtent) -> Result<(u16, u16), crate::HostError> {
    let size = extent.size();
    Ok((
        u16::try_from(size.width).map_err(|_| "Unsupported X11 client width")?,
        u16::try_from(size.height).map_err(|_| "Unsupported X11 client height")?,
    ))
}
