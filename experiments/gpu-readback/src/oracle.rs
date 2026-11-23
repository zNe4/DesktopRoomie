use std::time::Duration;

use desktop_probe::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameMetadata, FrameProducer,
    FrameRequest, HitRegion, Point, PreparedFrame, PresentationCapabilities, Rect, RowSpan, Size,
};
use sha2::{Digest, Sha256};

use crate::layout::{Layout, CHANNELS};
use crate::{PERIOD, PHASES};

// Compile the accepted producer source verbatim against the same presentation API.
#[path = "../../desktop-probe/src/presentation/synthetic.rs"]
mod synthetic;

const IDLE: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};

pub struct Oracle {
    pub frames: Vec<Reference>,
}

pub struct Reference {
    pub frame: PreparedFrame<Vec<u8>>,
    pub digest: String,
    pub coverage: Coverage,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Coverage {
    pub mask: Vec<u8>,
    pub spans: Vec<RowSpan>,
    pub bounds: Option<Rect>,
}

impl Oracle {
    pub fn new(layout: Layout) -> Result<Self, String> {
        let mut producer = synthetic::SyntheticProducer::new();
        let capabilities = producer.capabilities();
        let extent = capabilities.initial_extent();
        if extent.size() != Size::new(layout.width as u32, layout.height as u32)
            || capabilities.supported_intents() != [IDLE]
            || capabilities.frame_period() != Some(PERIOD)
        {
            return Err("accepted fixture extent, intent or period changed".into());
        }
        let mut frames = Vec::new();
        for phase in 0..PHASES {
            let frame = prepare(&mut producer, extent, PERIOD * phase as u32)?;
            let coverage = Coverage::from_bytes(layout, &frame.payload)?;
            if frame.metadata.extent() != extent
                || frame.metadata.visible_bounds() != coverage.bounds
                || frame.metadata.hit_region().spans() != coverage.spans
            {
                return Err(format!(
                    "CPU fixture phase {phase} metadata disagrees with its payload"
                ));
            }
            frames.push(Reference {
                digest: digest(&frame.payload),
                frame,
                coverage,
            });
        }
        Ok(Self { frames })
    }
}

fn prepare(
    producer: &mut synthetic::SyntheticProducer,
    extent: ClientExtent,
    elapsed: Duration,
) -> Result<PreparedFrame<Vec<u8>>, String> {
    let request = FrameRequest::new(
        FrameIdentity {
            generation: 1,
            sequence: 1 + (elapsed.as_millis() / PERIOD.as_millis()) as u64,
        },
        IDLE,
        extent,
        elapsed,
        Duration::ZERO,
    )
    .map_err(|error| format!("CPU fixture request: {error:?}"))?;
    let frame = producer
        .prepare_frame(&request)
        .map_err(|error| error.to_string())?;
    if frame.identity != request.identity() {
        return Err("CPU fixture returned a different frame identity".into());
    }
    Ok(frame)
}

impl Coverage {
    pub fn from_bytes(layout: Layout, bytes: &[u8]) -> Result<Self, String> {
        layout
            .check_packed(bytes)
            .map_err(|error| format!("packed BGRA8: {error:?}"))?;
        let mut mask = Vec::with_capacity(layout.width * layout.height);
        for (index, pixel) in bytes.as_chunks::<CHANNELS>().0.iter().enumerate() {
            if pixel[..3].iter().any(|&channel| channel > pixel[3]) {
                return Err(format!(
                    "non-premultiplied BGRA8 at ({}, {}): {pixel:?}",
                    index % layout.width,
                    index / layout.width
                ));
            }
            mask.push(u8::from(pixel[3] != 0));
        }
        let mut spans = Vec::new();
        for (y, row) in mask.chunks_exact(layout.width).enumerate() {
            let mut start = None;
            for (x, &covered) in row.iter().chain(&[0]).enumerate() {
                match (covered, start) {
                    (1, None) => start = Some(x),
                    (0, Some(left)) => {
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
            let left = spans.iter().map(|span| span.start_x).min().unwrap();
            let right = spans.iter().map(|span| span.end_x).max().unwrap();
            Rect::new(
                left,
                first.y,
                (right - left) as u32,
                (last.y + 1 - first.y) as u32,
            )
        });
        Ok(Self {
            mask,
            spans,
            bounds,
        })
    }
}

impl Reference {
    pub fn compare(&self, phase: usize, layout: Layout, actual: &[u8]) -> Result<Coverage, String> {
        layout
            .check_packed(actual)
            .map_err(|error| format!("phase {phase} readback size: {error:?}"))?;
        let expected = &self.frame.payload;
        let coverage = Coverage::from_bytes(layout, actual);
        if expected != actual {
            let first = expected
                .iter()
                .zip(actual)
                .position(|(a, b)| a != b)
                .unwrap();
            let mismatched_bytes = expected.iter().zip(actual).filter(|(a, b)| a != b).count();
            let expected_pixels = expected.as_chunks::<CHANNELS>().0;
            let actual_pixels = actual.as_chunks::<CHANNELS>().0;
            let mismatched_pixels = expected_pixels
                .iter()
                .zip(actual_pixels)
                .filter(|(a, b)| a != b)
                .count();
            let pixel = first / CHANNELS;
            let alpha_mask_equal = expected_pixels
                .iter()
                .zip(actual_pixels)
                .all(|(a, b)| (a[3] != 0) == (b[3] != 0));
            let coverage_report = match &coverage {
                Ok(coverage) => format!(
                    "bounds_equal={} spans_equal={}",
                    coverage.bounds == self.frame.metadata.visible_bounds(),
                    coverage.spans == self.frame.metadata.hit_region().spans()
                ),
                Err(error) => error.clone(),
            };
            return Err(format!(
                "phase {phase} mismatch at ({}, {}) channel {} expected={} actual={} mismatched_bytes={mismatched_bytes} mismatched_pixels={mismatched_pixels} cpu_sha256={} gpu_sha256={} alpha_mask_equal={alpha_mask_equal} {coverage_report}",
                pixel % layout.width, pixel / layout.width,
                ["B", "G", "R", "A"][first % CHANNELS], expected[first], actual[first],
                self.digest, digest(actual),
            ));
        }
        let coverage = coverage.map_err(|error| format!("phase {phase} {error}"))?;
        if coverage.mask != self.coverage.mask
            || coverage.bounds != self.frame.metadata.visible_bounds()
            || coverage.spans != self.frame.metadata.hit_region().spans()
        {
            return Err(format!(
                "phase {phase} alpha-mask, bounds or hit-span mismatch cpu_sha256={} gpu_sha256={}",
                self.digest,
                digest(actual)
            ));
        }
        Ok(coverage)
    }
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_period_preserves_all_phase_edges_and_wraps() {
        let oracle = Oracle::new(Layout::fixture().unwrap()).unwrap();
        let mut producer = synthetic::SyntheticProducer::new();
        let extent = producer.capabilities().initial_extent();
        for i in 0..10 {
            for offset in [Duration::ZERO, PERIOD - Duration::from_nanos(1)] {
                let actual = prepare(&mut producer, extent, PERIOD * i + offset).unwrap();
                let expected = &oracle.frames[i as usize % PHASES].frame;
                assert_eq!(actual.payload, expected.payload);
                assert_eq!(actual.metadata, expected.metadata);
            }
        }
    }

    #[test]
    fn accepted_coverage_has_recolor_hole_disjoint_and_empty() {
        let layout = Layout::fixture().unwrap();
        let oracle = Oracle::new(layout).unwrap();
        let [first, second, hole, disjoint, empty] = oracle.frames.as_slice() else {
            panic!("five phases")
        };
        assert_eq!(first.coverage, second.coverage);
        assert_ne!(first.frame.payload, second.frame.payload);
        let pixel = |frame: &Reference, x: usize, y: usize| -> [u8; 4] {
            let offset = y * layout.row + 4 * x;
            frame.frame.payload[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(pixel(first, 17, 40), [230, 115, 60, 255]);
        assert_eq!(pixel(hole, 61, 65), [0; 4]);
        assert_eq!(pixel(hole, 60, 65), [112, 45, 96, 128]);
        assert_eq!(pixel(disjoint, 113, 24), [90, 160, 240, 255]);
        assert_eq!(pixel(disjoint, 132, 101), [35, 95, 120, 128]);
        assert!(disjoint
            .coverage
            .spans
            .iter()
            .all(|span| !(58..101).contains(&span.y)));
        assert!(empty.frame.payload.iter().all(|&byte| byte == 0));
        assert!(empty.coverage.spans.is_empty());
        assert_eq!(empty.coverage.bounds, None);
    }

    #[test]
    fn detects_rgba_channel_swap_and_reports_hashes_and_counts() {
        let layout = Layout::fixture().unwrap();
        let oracle = Oracle::new(layout).unwrap();
        let phase = &oracle.frames[0];
        let mut swapped = phase.frame.payload.clone();
        for pixel in swapped.as_chunks_mut::<CHANNELS>().0 {
            pixel.swap(0, 2);
        }
        let error = phase.compare(0, layout, &swapped).unwrap_err();
        assert!(
            error.contains("phase 0 mismatch at (17, 40) channel B expected=230 actual=60"),
            "{error}"
        );
        assert!(error.contains("mismatched_bytes=6272 mismatched_pixels=3136"));
        assert!(error.contains("alpha_mask_equal=true bounds_equal=true spans_equal=true"));
        assert!(error.contains(&phase.digest));
        assert!(error.contains(&digest(&swapped)));
    }

    #[test]
    fn detects_inverted_rows_and_oversize_payload() {
        let layout = Layout::fixture().unwrap();
        let oracle = Oracle::new(layout).unwrap();
        let phase = &oracle.frames[3];
        let flipped: Vec<u8> = phase
            .frame
            .payload
            .chunks_exact(layout.row)
            .rev()
            .flatten()
            .copied()
            .collect();
        let error = phase.compare(3, layout, &flipped).unwrap_err();
        assert!(
            error.contains("alpha_mask_equal=false bounds_equal=false spans_equal=false"),
            "{error}"
        );
        assert!(phase
            .compare(3, layout, &[0; 102_401])
            .unwrap_err()
            .contains("phase 3 readback size"));
    }

    #[test]
    fn rejects_color_greater_than_alpha_including_transparent_pixels() {
        let layout = Layout::fixture().unwrap();
        let oracle = Oracle::new(layout).unwrap();
        let mut malformed = oracle.frames[4].frame.payload.clone();
        malformed[2] = 1;
        let error = oracle.frames[4].compare(4, layout, &malformed).unwrap_err();
        assert!(
            error.contains("non-premultiplied BGRA8 at (0, 0)"),
            "{error}"
        );
    }
}
