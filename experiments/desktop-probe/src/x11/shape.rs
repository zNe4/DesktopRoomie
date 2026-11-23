use super::native_dimensions;
use crate::presentation::HitRegion;
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as ShapeExt, SK, SO};
use x11rb::protocol::xproto::{ClipOrdering, Rectangle, Window};

/// Returns the full checked enabling sequence for the application freshness gate.
pub fn restore_body_input_shape(
    conn: &impl Connection,
    window: Window,
    region: &HitRegion,
) -> Result<u64, Box<dyn std::error::Error>> {
    restore_body_input_shape_with(window, region, |request| request.send(conn))
}

fn restore_body_input_shape_with(
    window: Window,
    region: &HitRegion,
    checked: impl FnOnce(InputShapeRequest) -> Result<u64, Box<dyn std::error::Error>>,
) -> Result<u64, Box<dyn std::error::Error>> {
    set_input_with(window, input_rectangles(region)?, checked)
}

pub fn empty_body_input_shape(
    conn: &impl Connection,
    window: Window,
) -> Result<u64, Box<dyn std::error::Error>> {
    set_input_with(window, vec![], |request| request.send(conn))
}

/// Preserve canonical row order and half-open coverage. Check exclusive
/// edges too, so signed X11 region coordinates cannot clip covered pixels.
pub(super) fn input_rectangles(
    region: &HitRegion,
) -> Result<Vec<Rectangle>, Box<dyn std::error::Error>> {
    native_dimensions(region.extent())?;
    region
        .spans()
        .iter()
        .map(|span| {
            i16::try_from(span.end_x).map_err(|_| "Unsupported X11 input region right edge")?;
            i16::try_from(span.y + 1).map_err(|_| "Unsupported X11 input region bottom edge")?;
            Ok(Rectangle {
                x: i16::try_from(span.start_x)?,
                y: i16::try_from(span.y)?,
                width: u16::try_from(span.end_x - span.start_x)?,
                height: 1,
            })
        })
        .collect()
}

struct InputShapeRequest {
    operation: SO,
    kind: SK,
    ordering: ClipOrdering,
    window: Window,
    x: i16,
    y: i16,
    rectangles: Vec<Rectangle>,
}
impl InputShapeRequest {
    fn send(self, conn: &impl Connection) -> Result<u64, Box<dyn std::error::Error>> {
        let cookie = conn.shape_rectangles(
            self.operation,
            self.kind,
            self.ordering,
            self.window,
            self.x,
            self.y,
            &self.rectangles,
        )?;
        let sequence = cookie.sequence_number();
        cookie.check()?;
        Ok(sequence)
    }
}
fn set_input_with(
    window: Window,
    rectangles: Vec<Rectangle>,
    checked: impl FnOnce(InputShapeRequest) -> Result<u64, Box<dyn std::error::Error>>,
) -> Result<u64, Box<dyn std::error::Error>> {
    checked(InputShapeRequest {
        operation: SO::SET,
        kind: SK::INPUT,
        ordering: ClipOrdering::UNSORTED,
        window,
        x: 0,
        y: 0,
        rectangles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::presentation::procedural::{Palette, ProceduralProducer};
    use crate::presentation::{
        Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameProducer, FrameRequest,
        Point, RowSpan, Size,
    };
    use std::time::Duration;

    fn procedural_frame(palette: Palette) -> crate::presentation::PreparedFrame<Vec<u8>> {
        let mut producer = ProceduralProducer::new(palette);
        let request = FrameRequest::new(
            FrameIdentity {
                generation: 0,
                sequence: 0,
            },
            ActionIntent {
                action: Action::Idle,
                facing: Facing::Neutral,
            },
            producer.capabilities().initial_extent(),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        producer.prepare_frame(&request).unwrap()
    }

    #[test]
    fn post_map_uses_the_same_checked_input_silhouette_as_initial_setup() {
        let frame = procedural_frame(Palette::Lavender);
        let region = frame.metadata.hit_region();
        let mut initial = None;
        restore_body_input_shape_with(200, region, |request| {
            initial = Some(request);
            Ok(70000)
        })
        .unwrap();
        let initial = initial.unwrap();
        for fail in [false, true] {
            let mut runtime = crate::ProbeRuntime::test_presented();
            let mut window = crate::tests::test_window();
            window.window = 200;
            runtime.body = crate::BodyAvailability::Ready;
            runtime.arm_map_input_sync(70001);
            let event = x11rb::protocol::xproto::MapNotifyEvent {
                response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
                sequence: 70001u64 as u16,
                event: 200,
                window: 200,
                override_redirect: false,
            };
            let result =
                runtime.synchronize_placed_map_input_with(&event, &window, 70001, |region| {
                    restore_body_input_shape_with(200, region, |request| {
                        assert_eq!(request.operation, SO::SET);
                        assert_eq!(request.kind, SK::INPUT);
                        assert_eq!(request.ordering, ClipOrdering::UNSORTED);
                        assert_eq!((request.window, request.x, request.y), (200, 0, 0));
                        let fields = |r: &Rectangle| (r.x, r.y, r.width, r.height);
                        assert_eq!(
                            request.rectangles.iter().map(fields).collect::<Vec<_>>(),
                            initial.rectangles.iter().map(fields).collect::<Vec<_>>()
                        );
                        if fail {
                            Err("checked post-map SHAPE failure".into())
                        } else {
                            Ok(140000)
                        }
                    })
                });
            if fail {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "checked post-map SHAPE failure"
                );
                assert_eq!(runtime.input_fence.enabled_after, None);
            } else {
                result.unwrap();
                assert_eq!(runtime.input_fence.enabled_after, Some(140000));
            }
        }
    }

    #[test]
    fn restored_shape_matches_every_original_silhouette_pixel_including_patch_and_padding() {
        let frame = procedural_frame(Palette::Lavender);
        let rects = input_rectangles(frame.metadata.hit_region()).unwrap();
        for y in -1..=160i16 {
            for x in -1..=160i16 {
                let hits = rects
                    .iter()
                    .filter(|r| {
                        x >= r.x
                            && y >= r.y
                            && i32::from(x) < i32::from(r.x) + i32::from(r.width)
                            && i32::from(y) < i32::from(r.y) + i32::from(r.height)
                    })
                    .count();
                assert_eq!(
                    hits,
                    usize::from(crate::geometry::is_in_interactive_silhouette(x, y)),
                    "{x},{y}"
                );
                let dx = i32::from(x) - 80;
                let dy = i32::from(y) - 80;
                let accepted = dx * dx + dy * dy <= 45 * 45
                    || ((15..=65).contains(&x) && (15..=65).contains(&y));
                assert_eq!(
                    hits,
                    usize::from(accepted),
                    "accepted silhouette at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn procedural_coverage_matches_legacy_geometry_and_x11_rectangles() {
        for palette in [Palette::Lavender, Palette::Coral] {
            let frame = procedural_frame(palette);
            let extent = frame.metadata.extent();
            let region = frame.metadata.hit_region();
            let rects = input_rectangles(region).unwrap();
            for y in -1..=160i16 {
                for x in -1..=160i16 {
                    let point = Point::new(i32::from(x), i32::from(y));
                    let legacy_hit = crate::geometry::is_in_interactive_silhouette(x, y);
                    let alpha_hit = extent.contains(point)
                        && frame.payload[(y as usize * 160 + x as usize) * 4 + 3] != 0;
                    let rect_hit = rects.iter().any(|r| {
                        x >= r.x
                            && y >= r.y
                            && point.x < i32::from(r.x) + i32::from(r.width)
                            && point.y < i32::from(r.y) + i32::from(r.height)
                    });
                    assert_eq!(alpha_hit, legacy_hit, "{palette:?} alpha at {point:?}");
                    assert_eq!(
                        region.contains(point),
                        legacy_hit,
                        "{palette:?} spans at {point:?}"
                    );
                    assert_eq!(rect_hit, legacy_hit, "X11 rectangles at {point:?}");
                }
            }
        }
    }

    #[test]
    fn empty_and_restored_requests_are_checked_exact_input_sets() {
        let frame = procedural_frame(Palette::Lavender);
        let empty = HitRegion::new(frame.metadata.extent(), []).unwrap();
        for region in [&empty, frame.metadata.hit_region()] {
            let rectangles = input_rectangles(region).unwrap();
            for fail in [false, true] {
                let result = restore_body_input_shape_with(200, region, |request| {
                    assert_eq!(request.operation, SO::SET);
                    assert_eq!(request.kind, SK::INPUT);
                    assert_eq!(request.ordering, ClipOrdering::UNSORTED);
                    assert_eq!((request.window, request.x, request.y), (200, 0, 0));
                    let fields = |r: &Rectangle| (r.x, r.y, r.width, r.height);
                    assert_eq!(
                        request.rectangles.iter().map(fields).collect::<Vec<_>>(),
                        rectangles.iter().map(fields).collect::<Vec<_>>()
                    );
                    if fail {
                        Err("SHAPE check failed".into())
                    } else {
                        Ok(70000)
                    }
                });
                if fail {
                    assert!(result.is_err());
                } else {
                    assert_eq!(result.unwrap(), 70000);
                }
            }
        }
    }

    #[test]
    fn canonical_spans_preserve_wire_order_half_open_edges_and_holes() {
        let extent = ClientExtent::new(Size::new(9, 4)).unwrap();
        let region = HitRegion::new(
            extent,
            [
                RowSpan {
                    y: 0,
                    start_x: 0,
                    end_x: 2,
                },
                RowSpan {
                    y: 0,
                    start_x: 3,
                    end_x: 9,
                },
                RowSpan {
                    y: 1,
                    start_x: 0,
                    end_x: 1,
                },
                RowSpan {
                    y: 1,
                    start_x: 8,
                    end_x: 9,
                },
                RowSpan {
                    y: 3,
                    start_x: 1,
                    end_x: 2,
                },
            ],
        )
        .unwrap();
        let rectangles = input_rectangles(&region).unwrap();
        let wire: Vec<_> = rectangles
            .iter()
            .map(|r| (r.x, r.y, r.width, r.height))
            .collect();
        assert_eq!(
            wire,
            [
                (0, 0, 2, 1),
                (3, 0, 6, 1),
                (0, 1, 1, 1),
                (8, 1, 1, 1),
                (1, 3, 1, 1)
            ]
        );
        for y in -1..=4 {
            for x in -1..=9 {
                let covered = rectangles
                    .iter()
                    .filter(|r| {
                        let (left, top) = (i32::from(r.x), i32::from(r.y));
                        (left..left + i32::from(r.width)).contains(&x)
                            && (top..top + i32::from(r.height)).contains(&y)
                    })
                    .count();
                assert_eq!(
                    covered,
                    usize::from(region.contains(Point::new(x, y))),
                    "{x},{y}"
                );
            }
        }
        let empty = HitRegion::new(extent, []).unwrap();
        restore_body_input_shape_with(42, &empty, |request| {
            assert!(request.rectangles.is_empty());
            assert_eq!((request.operation, request.kind), (SO::SET, SK::INPUT));
            Ok(140000)
        })
        .unwrap();
    }

    #[test]
    fn unrepresentable_region_edges_and_window_dimensions_send_nothing() {
        let edge = i32::from(i16::MAX);
        let extent = ClientExtent::new(Size::new(edge as u32 + 2, edge as u32 + 2)).unwrap();
        let representable = HitRegion::new(
            extent,
            [RowSpan {
                y: edge - 1,
                start_x: 0,
                end_x: edge,
            }],
        )
        .unwrap();
        let rectangles = input_rectangles(&representable).unwrap();
        assert_eq!(
            (rectangles[0].y, rectangles[0].width),
            (i16::MAX - 1, i16::MAX as u16)
        );
        for span in [
            RowSpan {
                y: 0,
                start_x: edge - 1,
                end_x: edge + 1,
            },
            RowSpan {
                y: 0,
                start_x: edge + 1,
                end_x: edge + 2,
            },
            RowSpan {
                y: edge,
                start_x: 0,
                end_x: 1,
            },
        ] {
            let region = HitRegion::new(extent, [span]).unwrap();
            assert!(restore_body_input_shape_with(42, &region, |_| panic!(
                "overflow must not send"
            ))
            .is_err());
        }
        for size in [
            Size::new(u32::from(u16::MAX) + 1, 1),
            Size::new(1, u32::from(u16::MAX) + 1),
        ] {
            let extent = ClientExtent::new(size).unwrap();
            assert!(native_dimensions(extent).is_err());
            let region = HitRegion::new(extent, []).unwrap();
            assert!(restore_body_input_shape_with(42, &region, |_| panic!(
                "overflow must not send"
            ))
            .is_err());
        }
    }
}
