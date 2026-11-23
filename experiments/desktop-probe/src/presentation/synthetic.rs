//! Fixed-canvas CPU fixture for the experimental X11 coverage cutover.

use std::time::Duration;

use super::{
    Action, ActionIntent, ClientExtent, Facing, FrameMetadata, FrameProducer, FrameRequest,
    HitRegion, Point, PreparedFrame, PresentationCapabilities, Rect, RowSpan, Size,
};
use crate::HostError;

const SIDE: usize = 160;
const PERIOD: Duration = Duration::from_millis(300);
const IDLE: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};

pub(crate) struct SyntheticProducer {
    capabilities: PresentationCapabilities,
}

impl SyntheticProducer {
    pub(crate) fn new() -> Self {
        let extent = ClientExtent::new(Size::new(SIDE as u32, SIDE as u32)).unwrap();
        Self {
            capabilities: PresentationCapabilities::new(extent, vec![IDLE], Some(PERIOD))
                .expect("valid fixed synthetic capabilities"),
        }
    }
}

impl FrameProducer for SyntheticProducer {
    type Payload = Vec<u8>;
    type Error = HostError;

    fn capabilities(&self) -> &PresentationCapabilities {
        &self.capabilities
    }

    fn prepare_frame(
        &mut self,
        request: &FrameRequest,
    ) -> Result<PreparedFrame<Vec<u8>>, HostError> {
        let extent = request.extent();
        if extent != self.capabilities.initial_extent() {
            return Err("Unsupported synthetic canvas extent".into());
        }
        if !self.capabilities.supports(request.intent()) {
            return Err("Unsupported synthetic presentation intent".into());
        }
        let phase = (request.active_elapsed().as_millis() / PERIOD.as_millis()) % 5;
        let mut pixels = vec![[0; 4]; SIDE * SIDE];
        match phase {
            0 => fill(&mut pixels, 17..73, 40..96, [230, 115, 60, 255]),
            1 => fill(&mut pixels, 17..73, 40..96, [70, 211, 150, 255]),
            2 => {
                fill(&mut pixels, 51..105, 53..110, [112, 45, 96, 128]);
                fill(&mut pixels, 61..86, 65..96, [0; 4]);
            }
            3 => {
                fill(&mut pixels, 113..137, 24..58, [90, 160, 240, 255]);
                fill(&mut pixels, 132..153, 101..131, [35, 95, 120, 128]);
            }
            _ => {}
        }

        // Derive coverage and bounds from exactly the pixels sent to the host.
        let mut spans = Vec::new();
        for (y, row) in pixels.as_chunks::<SIDE>().0.iter().enumerate() {
            let mut x = 0;
            while x < SIDE {
                if row[x][3] == 0 {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < SIDE && row[x][3] != 0 {
                    x += 1;
                }
                spans.push(RowSpan {
                    y: y as i32,
                    start_x: start as i32,
                    end_x: x as i32,
                });
            }
        }
        let bounds = spans.first().zip(spans.last()).map(|(first, last)| {
            let left = spans.iter().map(|span| span.start_x).min().unwrap();
            let right = spans.iter().map(|span| span.end_x).max().unwrap();
            Rect::new(
                left,
                first.y,
                (right - left) as u32,
                (last.y + 1 - first.y) as u32,
            )
        });
        let anchor = spans.first().map(|span| Point::new(span.start_x, span.y));
        let region = HitRegion::new(extent, spans)
            .map_err(|error| format!("Invalid synthetic coverage: {error:?}"))?;
        let metadata = FrameMetadata::new(extent, bounds, anchor, region)
            .map_err(|error| format!("Invalid synthetic metadata: {error:?}"))?;
        Ok(PreparedFrame {
            identity: request.identity(),
            metadata,
            payload: pixels.into_flattened(),
        })
    }
}

fn fill(
    pixels: &mut [[u8; 4]],
    x: std::ops::Range<usize>,
    y: std::ops::Range<usize>,
    color: [u8; 4],
) {
    for row in y {
        pixels[row * SIDE + x.start..row * SIDE + x.end].fill(color);
    }
}
