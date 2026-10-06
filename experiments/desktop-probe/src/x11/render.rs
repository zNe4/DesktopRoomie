use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Colormap, ConnectionExt as XprotoExt, CreateGCAux, Gcontext, ImageFormat, Pixmap, Window,
};

pub const WINDOW_WIDTH: u16 = 160;
pub const WINDOW_HEIGHT: u16 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTheme {
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
}

pub struct Renderer {
    pub pixmap: Pixmap,
    pub gc: Gcontext,
    pub width: u16,
    pub height: u16,
    pub theme: ColorTheme,
}

impl Renderer {
    pub fn new(
        conn: &impl Connection,
        window: Window,
        _colormap: Colormap,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let width = WINDOW_WIDTH;
        let height = WINDOW_HEIGHT;
        let theme = ColorTheme::Lavender;

        // 1. Create depth-32 Pixmap for double buffering
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(32, pixmap, window, width, height)?
            .check()?;

        // 2. Create GC
        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &CreateGCAux::new())?.check()?;

        // 3. Generate 32-bit ARGB image buffer
        let buffer = generate_test_body_pattern(width as usize, height as usize, theme);

        // 4. Upload buffer into Pixmap
        conn.put_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            gc,
            width,
            height,
            0,
            0,
            0,
            32,
            &buffer,
        )?
        .check()?;

        Ok(Self {
            pixmap,
            gc,
            width,
            height,
            theme,
        })
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
        self.theme = match self.theme {
            ColorTheme::Lavender => ColorTheme::Coral,
            ColorTheme::Coral => ColorTheme::Lavender,
        };

        let buffer =
            generate_test_body_pattern(self.width as usize, self.height as usize, self.theme);

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
            &buffer,
        )?
        .check()?;

        self.paint(conn, window)?;
        Ok(self.theme)
    }
}

/// Generates a 160x160 little-endian premultiplied ARGB buffer:
/// Byte order in memory: [B, G, R, A]
fn generate_test_body_pattern(width: usize, height: usize, theme: ColorTheme) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            let dx = (x as f32) - 80.0;
            let dy = (y as f32) - 80.0;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= 45.0 {
                // Check if this pixel is an eye/accent dot
                let is_left_eye = (x as i32 - 68).pow(2) + (y as i32 - 75).pow(2) <= 16;
                let is_right_eye = (x as i32 - 92).pow(2) + (y as i32 - 75).pow(2) <= 16;

                if is_left_eye || is_right_eye {
                    // Accent: White dots (A=255, R=255, G=255, B=255)
                    data[idx] = 255;
                    data[idx + 1] = 255;
                    data[idx + 2] = 255;
                    data[idx + 3] = 255;
                } else {
                    match theme {
                        ColorTheme::Lavender => {
                            // Vibrant Lavender/Purple (A=255, R=165, G=95, B=225)
                            data[idx] = 225; // B
                            data[idx + 1] = 95; // G
                            data[idx + 2] = 165; // R
                            data[idx + 3] = 255; // A
                        }
                        ColorTheme::Coral => {
                            // Warm Coral/Orange-Pink (A=255, R=240, G=110, B=80)
                            data[idx] = 80; // B
                            data[idx + 1] = 110; // G
                            data[idx + 2] = 240; // R
                            data[idx + 3] = 255; // A
                        }
                    }
                }
            } else if (15..=65).contains(&x) && (15..=65).contains(&y) {
                // Translucent test patch: 50% opacity Cyan (A=128, R=0, G=200, B=255)
                // Premultiplied: R=0, G=100, B=128, A=128
                data[idx] = 128; // B
                data[idx + 1] = 100; // G
                data[idx + 2] = 0; // R
                data[idx + 3] = 128; // A
            } else {
                // Empty region: Genuinely transparent (A=0, R=0, G=0, B=0)
                data[idx] = 0;
                data[idx + 1] = 0;
                data[idx + 2] = 0;
                data[idx + 3] = 0;
            }
        }
    }

    data
}
