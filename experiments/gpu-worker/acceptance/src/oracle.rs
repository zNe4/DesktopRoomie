//! Independent CPU fixture, compiled verbatim as in accepted B1. No WGSL input.
use desktop_probe::presentation::*;
use std::time::Duration;

#[path = "../../../desktop-probe/src/presentation/synthetic.rs"]
mod synthetic;

pub const SAMPLE_MS: [u64; 5] = [0, 300, 600, 900, 1200];
pub const BYTES: usize = 102_400;
pub const FIXTURE_SHA256: &str = "9bcca46707113b45f897e2ec688e4ff2467efb14d4416335a8d5729238ebb099";

pub fn request(sequence: u64, elapsed: Duration) -> FrameRequest {
    FrameRequest::new(
        FrameIdentity {
            generation: 1,
            sequence,
        },
        ActionIntent {
            action: Action::Idle,
            facing: Facing::Neutral,
        },
        ClientExtent::new(Size::new(160, 160)).unwrap(),
        elapsed,
        Duration::ZERO,
    )
    .unwrap()
}

pub fn expected(request: &FrameRequest) -> Result<PreparedFrame<Vec<u8>>, String> {
    synthetic::SyntheticProducer::new()
        .prepare_frame(request)
        .map_err(|e| e.to_string())
}

/// Derives only observed geometry from returned pixels. Expected geometry and
/// pixels come independently from the accepted producer, never this scan.
pub fn metadata(bytes: &[u8]) -> Result<FrameMetadata, String> {
    if bytes.len() != BYTES {
        return Err("expected exactly 102400 BGRA8 bytes".into());
    }
    if bytes
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[..3].iter().any(|c| *c > p[3]))
    {
        return Err("invalid premultiplied alpha".into());
    }
    let mut spans = Vec::new();
    for y in 0..160 {
        let mut start = None;
        for x in 0..=160 {
            let covered = x < 160 && bytes[(y * 160 + x) * 4 + 3] != 0;
            match (start, covered) {
                (None, true) => start = Some(x),
                (Some(left), false) => {
                    spans.push(RowSpan {
                        y: y as i32,
                        start_x: left as i32,
                        end_x: x as i32,
                    });
                    start = None;
                }
                _ => {}
            }
        }
    }
    let bounds = spans.first().zip(spans.last()).map(|(first, last)| {
        let left = spans.iter().map(|s| s.start_x).min().unwrap();
        let right = spans.iter().map(|s| s.end_x).max().unwrap();
        Rect::new(
            left,
            first.y,
            (right - left) as u32,
            (last.y - first.y + 1) as u32,
        )
    });
    let anchor = spans.first().map(|s| Point::new(s.start_x, s.y));
    let extent = ClientExtent::new(Size::new(160, 160)).unwrap();
    FrameMetadata::new(
        extent,
        bounds,
        anchor,
        HitRegion::new(extent, spans).map_err(|e| format!("{e:?}"))?,
    )
    .map_err(|e| format!("{e:?}"))
}

pub fn validate(request: &FrameRequest, frame: &PreparedFrame<Vec<u8>>) -> Result<(), String> {
    let observed = metadata(&frame.payload)?;
    let cpu = expected(request)?;
    if request.extent().size() != Size::new(160, 160) || frame.identity != request.identity() {
        return Err("frame extent or identity mismatch".into());
    }
    if frame.metadata != observed || frame.metadata != cpu.metadata {
        return Err("canonical hit spans, bounds, anchor or extent mismatch".into());
    }
    if frame.payload != cpu.payload {
        return Err("exact CPU pixel/alpha mask mismatch".into());
    }
    Ok(())
}
