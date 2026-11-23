use super::resource::{cleanup_all, OwnedResource};
use crate::presentation::procedural::{Palette, ProceduralProducer};
use crate::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameProducer, FrameRequest, Size,
};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Colormap, ConnectionExt as XprotoExt, CreateGCAux, Gcontext, ImageFormat, Pixmap, Window,
};

pub const WINDOW_WIDTH: u16 = 160;
pub const WINDOW_HEIGHT: u16 = 160;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ColorTheme {
    #[default]
    Lavender,
    Coral,
}

impl ColorTheme {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Lavender => "Lavender (Default)",
            Self::Coral => "Coral (Clicked)",
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::Lavender => Self::Coral,
            Self::Coral => Self::Lavender,
        }
    }

    fn palette(self) -> Palette {
        match self {
            Self::Lavender => Palette::Lavender,
            Self::Coral => Palette::Coral,
        }
    }
}

pub struct Renderer {
    pub gc_resource: OwnedResource,
    pub pixmap_resource: OwnedResource,

    pub pixmap: Pixmap,
    pub gc: Gcontext,
    pub width: u16,
    pub height: u16,
    pub theme: ColorTheme,
    frame_identity: FrameIdentity,
}

impl Renderer {
    pub fn new(
        conn: &impl Connection,
        window: Window,
        _colormap: Colormap,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let width = WINDOW_WIDTH;
        let height = WINDOW_HEIGHT;
        let theme = ColorTheme::default();

        // 1. Create depth-32 Pixmap for double buffering
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(32, pixmap, window, width, height)?
            .check()?;

        // Keep the successfully allocated pixmap owned during all later initialization.
        let pixmap_resource = OwnedResource::default();
        let gc_result = (|| -> Result<_, Box<dyn std::error::Error>> {
            let gc = conn.generate_id()?;
            conn.create_gc(gc, pixmap, &CreateGCAux::new())?.check()?;
            Ok(gc)
        })();
        let gc = match gc_result {
            Ok(gc) => gc,
            Err(error) => {
                eprintln!("[ERROR] Renderer GC creation failed: {error}");
                let _ = cleanup_all([Box::new(|| {
                    pixmap_resource.release_with(|| {
                        conn.free_pixmap(pixmap)?.check()?;
                        Ok(())
                    })
                })]);
                return Err(error);
            }
        };
        let renderer = Self {
            pixmap_resource,
            gc_resource: OwnedResource::default(),
            pixmap,
            gc,
            width,
            height,
            theme,
            frame_identity: FrameIdentity {
                generation: 0,
                sequence: 0,
            },
        };
        if let Err(error) = renderer.upload_frame(conn) {
            eprintln!("[ERROR] Renderer upload failed: {error}");
            if let Err(cleanup) = renderer.destroy(conn) {
                eprintln!("[ERROR] Partial renderer cleanup: {cleanup}");
            }
            return Err(error);
        }
        Ok(renderer)
    }

    /// Blit from the double-buffered Pixmap to the window on Expose
    pub fn paint(
        &self,
        conn: &impl Connection,
        window: Window,
    ) -> Result<(), Box<dyn std::error::Error>> {
        conn.copy_area(
            self.pixmap,
            window,
            self.gc,
            0,
            0,
            0,
            0,
            self.width,
            self.height,
        )?
        .check()?;
        conn.flush()?;
        Ok(())
    }

    /// Toggles the body color palette on mouse click for immediate visual feedback
    pub fn toggle_color(
        &mut self,
        conn: &impl Connection,
        window: Window,
    ) -> Result<ColorTheme, Box<dyn std::error::Error>> {
        self.theme = self.theme.toggled();
        self.frame_identity = self
            .frame_identity
            .checked_next()
            .ok_or("Renderer frame sequence exhausted")?;

        self.upload_frame(conn)?;
        self.paint(conn, window)?;
        Ok(self.theme)
    }

    fn upload_frame(&self, conn: &impl Connection) -> Result<(), Box<dyn std::error::Error>> {
        let extent = ClientExtent::new(Size::new(self.width.into(), self.height.into()))
            .map_err(|error| format!("Invalid renderer frame extent: {error:?}"))?;
        let request = FrameRequest::new(
            self.frame_identity,
            ActionIntent {
                action: Action::Idle,
                facing: Facing::Neutral,
            },
            extent,
            Duration::ZERO,
            Duration::ZERO,
        )
        .map_err(|error| format!("Invalid renderer frame request: {error:?}"))?;
        let frame = ProceduralProducer::new(self.theme.palette()).prepare_frame(&request)?;

        conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.pixmap,
            self.gc,
            self.width,
            self.height,
            0,
            0,
            0,
            32,
            &frame.payload,
        )?
        .check()?;

        Ok(())
    }

    /// Releases server-side GC and Pixmap resources, checking requests and reporting failures
    pub fn destroy(&self, conn: &impl Connection) -> Result<(), Box<dyn std::error::Error>> {
        cleanup_all([
            Box::new(|| {
                self.gc_resource.release_with(|| {
                    conn.free_gc(self.gc)?.check()?;
                    Ok(())
                })
            }),
            Box::new(|| {
                self.pixmap_resource.release_with(|| {
                    conn.free_pixmap(self.pixmap)?.check()?;
                    Ok(())
                })
            }),
            Box::new(|| {
                conn.flush()?;
                Ok(())
            }),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_theme_starts_lavender_and_alternates_adapter_palettes() {
        let initial = ColorTheme::default();
        let clicked = initial.toggled();
        let clicked_again = clicked.toggled();
        assert_eq!(
            (initial, clicked, clicked_again),
            (ColorTheme::Lavender, ColorTheme::Coral, initial)
        );
        assert_eq!(
            [
                initial.palette(),
                clicked.palette(),
                clicked_again.palette()
            ],
            [Palette::Lavender, Palette::Coral, Palette::Lavender]
        );
    }
}
