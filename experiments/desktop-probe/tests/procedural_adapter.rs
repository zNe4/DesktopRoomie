use std::time::Duration;

use desktop_probe::presentation::procedural::{Palette, ProceduralError, ProceduralProducer};
use desktop_probe::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameProducer, FrameRequest, Point,
    PreparedFrame, Rect, Size,
};

const IDLE: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};
const FIRST: FrameIdentity = FrameIdentity {
    generation: 3,
    sequence: 5,
};
const LAVENDER_BGRA: [u8; 4] = [225, 95, 165, 255];
const CORAL_BGRA: [u8; 4] = [80, 110, 240, 255];

fn extent() -> ClientExtent {
    ClientExtent::new(Size::new(160, 160)).unwrap()
}

fn request(identity: FrameIdentity, intent: ActionIntent, extent: ClientExtent) -> FrameRequest {
    FrameRequest::new(identity, intent, extent, Duration::ZERO, Duration::ZERO).unwrap()
}

fn render(palette: Palette) -> PreparedFrame<Vec<u8>> {
    ProceduralProducer::new(palette)
        .prepare_frame(&request(FIRST, IDLE, extent()))
        .unwrap()
}

// Frozen B1 render.rs (fd394a5). Keep its floating-point distance test and
// conditional order independent of the adapter's integer rasterizer.
fn frozen_body_pattern(body_color: [u8; 4]) -> Vec<u8> {
    let mut data = vec![0; 160 * 160 * 4];
    for y in 0..160 {
        for x in 0..160 {
            let idx = (y * 160 + x) as usize * 4;
            let dx = x as f32 - 80.0;
            let dy = y as f32 - 80.0;
            let dist = (dx * dx + dy * dy).sqrt();

            let bgra = if dist <= 45.0 {
                let is_left_eye = (x - 68i32).pow(2) + (y - 75i32).pow(2) <= 16;
                let is_right_eye = (x - 92i32).pow(2) + (y - 75i32).pow(2) <= 16;
                if is_left_eye || is_right_eye {
                    [255, 255, 255, 255]
                } else {
                    body_color
                }
            } else if (15..=65).contains(&x) && (15..=65).contains(&y) {
                [128, 100, 0, 128]
            } else {
                [0, 0, 0, 0]
            };
            data[idx..idx + 4].copy_from_slice(&bgra);
        }
    }
    data
}

fn assert_frozen_parity(palette: Palette, body_color: [u8; 4]) {
    let frame = render(palette);
    let expected = frozen_body_pattern(body_color);
    assert_eq!(expected.len(), 102_400);
    assert_eq!(frame.payload.len(), expected.len());
    for (index, (actual, expected)) in frame.payload.chunks(4).zip(expected.chunks(4)).enumerate() {
        assert_eq!(
            actual,
            expected,
            "{palette:?} at ({}, {})",
            index % 160,
            index / 160
        );
    }
}

#[test]
fn lavender_matches_the_frozen_buffer_byte_for_byte() {
    assert_frozen_parity(Palette::Lavender, LAVENDER_BGRA);
}

#[test]
fn coral_matches_the_frozen_buffer_byte_for_byte() {
    assert_frozen_parity(Palette::Coral, CORAL_BGRA);
}

#[test]
fn pixels_are_premultiplied_bgra_with_zeroed_transparent_channels() {
    for (palette, body_color) in [
        (Palette::Lavender, LAVENDER_BGRA),
        (Palette::Coral, CORAL_BGRA),
    ] {
        let frame = render(palette);
        let mut visible = 0;
        let mut transparent = 0;
        for pixel in frame.payload.chunks(4) {
            assert!(pixel[..3].iter().all(|channel| *channel <= pixel[3]));
            if pixel[3] == 0 {
                assert_eq!(pixel, [0, 0, 0, 0]);
                transparent += 1;
            } else {
                visible += 1;
            }
        }
        assert_eq!((visible, transparent), (8_441, 17_159));

        for (x, y, expected) in [
            (15, 15, [128, 100, 0, 128]),
            (15, 65, [128, 100, 0, 128]),
            (65, 65, body_color), // Opaque body takes precedence in the overlap.
            (125, 80, body_color),
            (126, 80, [0, 0, 0, 0]),
            (68, 71, [255, 255, 255, 255]),
            (96, 75, [255, 255, 255, 255]),
        ] {
            let start = (y * 160 + x) * 4;
            assert_eq!(
                frame.payload[start..start + 4],
                expected,
                "{palette:?} at ({x}, {y})"
            );
        }
    }
}

#[test]
fn hit_spans_are_canonical_and_match_every_alpha_pixel() {
    for palette in [Palette::Lavender, Palette::Coral] {
        let frame = render(palette);
        let metadata = frame.metadata;
        let region = metadata.hit_region();
        let spans = region.spans();
        assert_eq!(metadata.visible_bounds(), Some(Rect::new(15, 15, 111, 111)));
        assert_eq!(metadata.anchor(), None);
        assert_eq!(region.extent(), extent());
        assert_eq!(spans.len(), 114);
        assert!(spans.iter().all(|span| span.start_x < span.end_x));
        for pair in spans.windows(2) {
            let [first, second] = [pair[0], pair[1]];
            assert!(first.y < second.y || (first.y == second.y && first.end_x < second.start_x));
        }
        for (index, pixel) in frame.payload.chunks(4).enumerate() {
            let point = Point::new((index % 160) as i32, (index / 160) as i32);
            assert_eq!(
                region.contains(point),
                pixel[3] != 0,
                "{palette:?} at {point:?}"
            );
        }
    }
}

#[test]
fn capabilities_are_exactly_static_neutral_idle_at_160_square() {
    for palette in [Palette::Lavender, Palette::Coral] {
        let producer = ProceduralProducer::new(palette);
        let capabilities = producer.capabilities();
        assert_eq!(capabilities.initial_extent(), extent());
        assert_eq!(capabilities.supported_intents(), &[IDLE]);
        assert!(capabilities.supports(IDLE));
        assert_eq!(capabilities.frame_period(), None);
    }
}

#[test]
fn rejects_unsupported_widths_and_heights() {
    let mut producer = ProceduralProducer::new(Palette::Lavender);
    for size in [
        Size::new(159, 160),
        Size::new(160, 161),
        Size::new(i32::MAX as u32, 160),
    ] {
        let extent = ClientExtent::new(size).unwrap();
        assert_eq!(
            producer
                .prepare_frame(&request(FIRST, IDLE, extent))
                .unwrap_err(),
            ProceduralError::UnsupportedExtent(extent)
        );
    }
}

#[test]
fn rejects_all_other_action_facing_pairs_explicitly() {
    let mut producer = ProceduralProducer::new(Palette::Coral);
    for action in [Action::Idle, Action::Walk] {
        for facing in [
            Facing::Neutral,
            Facing::FrontRight,
            Facing::FrontLeft,
            Facing::BackRight,
            Facing::BackLeft,
        ] {
            let intent = ActionIntent { action, facing };
            if intent == IDLE {
                continue;
            }
            assert!(!producer.capabilities().supports(intent));
            assert_eq!(
                producer
                    .prepare_frame(&request(FIRST, intent, extent()))
                    .unwrap_err(),
                ProceduralError::UnsupportedIntent(intent)
            );
        }
    }
}

#[test]
fn preserves_request_identity_and_extent_and_ignores_active_time() {
    for palette in [Palette::Lavender, Palette::Coral] {
        let mut producer = ProceduralProducer::new(palette);
        let first = producer
            .prepare_frame(&request(FIRST, IDLE, extent()))
            .unwrap();
        let later_identity = FrameIdentity {
            generation: u64::MAX,
            sequence: u64::MAX,
        };
        let later_request = FrameRequest::new(
            later_identity,
            IDLE,
            extent(),
            Duration::MAX,
            Duration::from_secs(17),
        )
        .unwrap();
        let later = producer.prepare_frame(&later_request).unwrap();
        assert_eq!(first.identity, FIRST);
        assert_eq!(first.metadata.extent(), extent());
        assert_eq!(later.identity, later_request.identity());
        assert_eq!(later.metadata.extent(), later_request.extent());
        assert_eq!(first.metadata, later.metadata);
        assert!(
            first.payload == later.payload,
            "{palette:?} payload changed with active time"
        );
    }
}
