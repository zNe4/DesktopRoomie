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

fn validate_request_size(
    bytes: usize,
    maximum: usize,
    request: &str,
) -> Result<(), crate::HostError> {
    let wire_bytes = if bytes / 4 > usize::from(u16::MAX) {
        bytes.checked_add(4).ok_or("X11 request size overflow")? // BIG-REQUESTS length field
    } else {
        bytes
    };
    if wire_bytes > maximum {
        return Err(format!(
            "{request} request requires {wire_bytes} bytes; X11 limit is {maximum}"
        )
        .into());
    }
    Ok(())
}
