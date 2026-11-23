//! Static procedural body used by the standalone desktop probe.

use std::fmt;

use super::{
    Action, ActionIntent, ClientExtent, Facing, FrameMetadata, FrameProducer, FrameRequest,
    HitRegion, PreparedFrame, PresentationCapabilities, Rect, RowSpan, Size, ValidationError,
};

const SIZE: Size = Size::new(160, 160);
const IDLE: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Lavender,
    Coral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProceduralError {
    UnsupportedExtent(ClientExtent),
    UnsupportedIntent(ActionIntent),
    InvalidMetadata(ValidationError),
}

impl fmt::Display for ProceduralError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedExtent(extent) => {
                let size = extent.size();
                write!(
                    formatter,
                    "unsupported procedural extent {}x{}",
                    size.width, size.height
                )
            }
            Self::UnsupportedIntent(intent) => {
                write!(formatter, "unsupported procedural intent {intent:?}")
            }
            Self::InvalidMetadata(error) => {
                write!(formatter, "invalid procedural frame metadata: {error:?}")
            }
        }
    }
}

impl std::error::Error for ProceduralError {}

pub struct ProceduralProducer {
    capabilities: PresentationCapabilities,
    palette: Palette,
}

impl ProceduralProducer {
    pub fn new(palette: Palette) -> Self {
        let extent = ClientExtent::new(SIZE).expect("valid fixed procedural extent");
        let capabilities = PresentationCapabilities::new(extent, vec![IDLE], None)
            .expect("valid fixed procedural capabilities");
        Self {
            capabilities,
            palette,
        }
    }
}

impl FrameProducer for ProceduralProducer {
    /// Tightly packed, row-major premultiplied BGRA8 bytes.
    type Payload = Vec<u8>;
    type Error = ProceduralError;

    fn capabilities(&self) -> &PresentationCapabilities {
        &self.capabilities
    }

    fn prepare_frame(
        &mut self,
        request: &FrameRequest,
    ) -> Result<PreparedFrame<Self::Payload>, Self::Error> {
        let extent = request.extent();
        if extent != self.capabilities.initial_extent() {
            return Err(ProceduralError::UnsupportedExtent(extent));
        }
        if !self.capabilities.supports(request.intent()) {
            return Err(ProceduralError::UnsupportedIntent(request.intent()));
        }

        let body_color = match self.palette {
            Palette::Lavender => [225, 95, 165, 255],
            Palette::Coral => [80, 110, 240, 255],
        };
        let width = SIZE.width as i32;
        let height = SIZE.height as i32;
        let mut payload = Vec::with_capacity(width as usize * height as usize * 4);
        let mut spans = Vec::new();

        for y in 0..height {
            let mut start_x = None;
            for x in 0..width {
                let pixel = body_pixel(x, y, body_color);
                payload.extend_from_slice(&pixel);
                if pixel[3] != 0 {
                    start_x.get_or_insert(x);
                } else if let Some(start_x) = start_x.take() {
                    spans.push(RowSpan {
                        y,
                        start_x,
                        end_x: x,
                    });
                }
            }
            if let Some(start_x) = start_x {
                spans.push(RowSpan {
                    y,
                    start_x,
                    end_x: width,
                });
            }
        }

        let hit_region = HitRegion::new(extent, spans).map_err(ProceduralError::InvalidMetadata)?;
        let metadata =
            FrameMetadata::new(extent, Some(Rect::new(15, 15, 111, 111)), None, hit_region)
                .map_err(ProceduralError::InvalidMetadata)?;
        Ok(PreparedFrame {
            identity: request.identity(),
            metadata,
            payload,
        })
    }
}

fn body_pixel(x: i32, y: i32, body_color: [u8; 4]) -> [u8; 4] {
    if in_circle(x - 80, y - 80, 45) {
        if in_circle(x - 68, y - 75, 4) || in_circle(x - 92, y - 75, 4) {
            [255, 255, 255, 255]
        } else {
            body_color
        }
    } else if (15..=65).contains(&x) && (15..=65).contains(&y) {
        [128, 100, 0, 128]
    } else {
        [0, 0, 0, 0]
    }
}

fn in_circle(dx: i32, dy: i32, radius: i32) -> bool {
    dx * dx + dy * dy <= radius * radius
}
