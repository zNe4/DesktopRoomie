use super::{
    native_dimensions,
    resource::{cleanup_all, OwnedResource},
    shape::{input_rectangles, validate_input_request_size},
};
#[cfg(test)]
use crate::presentation::procedural::{Palette, ProceduralProducer};
#[cfg(test)]
use crate::presentation::{Action, Facing, FrameProducer};
use crate::presentation::{
    ActionIntent, ClientExtent, FrameIdentity, FrameMetadata, FrameRequest, PreparedFrame,
    Publication, PublicationError, PublicationEvent, PublicationState,
};
use crate::HostError;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, CreateGCAux, Gcontext, ImageFormat, Pixmap, Rectangle, Window,
};

const INITIAL_IDENTITY: FrameIdentity = FrameIdentity {
    generation: 0,
    sequence: 0,
};

pub(crate) struct CommittedFrameGeometry {
    pub identity: FrameIdentity,
    pub metadata: FrameMetadata,
}

pub(crate) enum PreparedPublication {
    /// Admission declined before publication state or native mutation.
    Skipped,
    Committed(Option<u64>),
}

/// Owner input obligations during the optional changing-coverage publication.
pub(crate) trait CoverageCutover {
    fn maximum_request_bytes(&self) -> usize;
    fn block_input(&mut self);
    fn empty_and_release(&mut self) -> Result<(), HostError>;
    fn enable(&mut self, rectangles: Vec<Rectangle>) -> Result<u64, HostError>;
    #[cfg(feature = "managed-supervisor-fixture")]
    fn input_ready_for_admission(&self) -> bool {
        false
    }
}

/// Geometry for one fixed X11 presentation target. The publication tracker
/// decides whether the retained metadata is still eligible for use.
pub(crate) struct FrameGeometry {
    initial_extent: ClientExtent,
    publication: Publication,
    committed: Option<CommittedFrameGeometry>,
}

impl FrameGeometry {
    pub fn new(initial_extent: ClientExtent) -> Self {
        Self {
            initial_extent,
            publication: Publication::new(INITIAL_IDENTITY.generation),
            committed: None,
        }
    }

    pub fn initial_extent(&self) -> ClientExtent {
        self.initial_extent
    }

    pub fn committed(&self) -> Option<&CommittedFrameGeometry> {
        self.committed
            .as_ref()
            .filter(|frame| self.publication.current_committed() == Some(frame.identity))
    }

    pub fn require_committed(&self) -> Result<&CommittedFrameGeometry, HostError> {
        self.committed()
            .ok_or_else(|| "Body has no committed frame geometry".into())
    }

    pub fn invalidate(&mut self) {
        // Invalidation also revokes authority if the generation is exhausted.
        let _ = self.publication.invalidate();
    }

    pub fn publication_uncertain(&self) -> bool {
        self.publication.visibility_uncertain()
    }

    pub(crate) fn frame_request(
        &self,
        intent: ActionIntent,
        elapsed: Duration,
        delta: Duration,
    ) -> Result<FrameRequest, HostError> {
        if self.publication_uncertain() {
            return Err("Renderer publication remains unsettled".into());
        }
        let (identity, extent) = match self.committed() {
            Some(frame) => (
                frame
                    .identity
                    .checked_next()
                    .ok_or("Renderer frame sequence exhausted")?,
                frame.metadata.extent(),
            ),
            None if self.committed.is_none()
                && self.publication.state(INITIAL_IDENTITY) != Ok(PublicationState::Stale) =>
            {
                (INITIAL_IDENTITY, self.initial_extent)
            }
            None => return Err("Renderer frame authority was invalidated".into()),
        };
        FrameRequest::new(identity, intent, extent, elapsed, delta)
            .map_err(|error| format!("Invalid renderer frame request: {error:?}").into())
    }

    #[cfg(feature = "managed-supervisor-fixture")]
    pub(crate) fn matches_request(&self, request: &FrameRequest) -> bool {
        self.frame_request(
            request.intent(),
            request.active_elapsed(),
            request.active_delta(),
        )
        .is_ok_and(|current| current == *request)
    }

    #[cfg(test)]
    pub(crate) fn publish_with(
        &mut self,
        request: &FrameRequest,
        frame: PreparedFrame<Vec<u8>>,
        upload: impl FnOnce((u16, u16), &[u8]) -> Result<(), HostError>,
        blit: impl FnOnce((u16, u16)) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        self.publish_with_cutover(request, frame, upload, blit, None)?;
        Ok(())
    }

    pub(crate) fn publish_with_cutover(
        &mut self,
        request: &FrameRequest,
        frame: PreparedFrame<Vec<u8>>,
        upload: impl FnOnce((u16, u16), &[u8]) -> Result<(), HostError>,
        blit: impl FnOnce((u16, u16)) -> Result<(), HostError>,
        cutover: Option<&mut dyn CoverageCutover>,
    ) -> Result<Option<u64>, HostError> {
        match self.publish_with_admission(request, frame, upload, blit, cutover, |_, _| Ok(true))? {
            PreparedPublication::Committed(sequence) => Ok(sequence),
            PreparedPublication::Skipped => unreachable!("CPU admission is unconditional"),
        }
    }

    /// Final owner eligibility/freshness check after validation, before any
    /// publication state or native input/pixel mutation. The CPU path has no
    /// additional admission policy and performs no additional clock read.
    pub(crate) fn publish_with_admission(
        &mut self,
        request: &FrameRequest,
        frame: PreparedFrame<Vec<u8>>,
        upload: impl FnOnce((u16, u16), &[u8]) -> Result<(), HostError>,
        blit: impl FnOnce((u16, u16)) -> Result<(), HostError>,
        cutover: Option<&mut dyn CoverageCutover>,
        admit: impl FnOnce(&Self, Option<&dyn CoverageCutover>) -> Result<bool, HostError>,
    ) -> Result<PreparedPublication, HostError> {
        if request.extent() != self.initial_extent {
            return Err("Runtime frame extent changes are unsupported".into());
        }
        if frame.identity != request.identity() {
            return Err(publication_error(PublicationError::IdentityMismatch));
        }
        if frame.metadata.extent() != request.extent() {
            return Err(publication_error(PublicationError::ExtentMismatch));
        }
        let dimensions = native_dimensions(frame.metadata.extent())?;
        if cutover.is_some()
            && (i16::try_from(dimensions.0).is_err() || i16::try_from(dimensions.1).is_err())
        {
            return Err("Animated canvas exceeds X11 signed coordinate limits".into());
        }
        let previous = self.committed();
        let changed = previous.is_some_and(|committed| {
            committed.metadata.hit_region() != frame.metadata.hit_region()
        });
        if changed && cutover.is_none() {
            return Err("Runtime frame coverage changes are unsupported".into());
        }
        // Validate coverage before the first commit or a checked cutover.
        let rectangles = (previous.is_none() || changed)
            .then(|| input_rectangles(frame.metadata.hit_region()))
            .transpose()?;
        validate_bgra8(dimensions, &frame)?;
        if let Some(policy) = cutover.as_ref() {
            if frame.metadata.hit_region().spans().is_empty()
                && frame.metadata.visible_bounds().is_some()
            {
                return Err("Transparent animated frame must omit visible bounds".into());
            }
            let expected = self.frame_request(
                request.intent(),
                request.active_elapsed(),
                request.active_delta(),
            )?;
            if request.identity() != expected.identity() {
                return Err(publication_error(PublicationError::IdentityMismatch));
            }
            let maximum = policy.maximum_request_bytes();
            if let Some(rectangles) = &rectangles {
                validate_input_request_size(rectangles, maximum)?;
            }
            let image_bytes = frame
                .payload
                .len()
                .checked_add(24)
                .ok_or("PutImage request size overflow")?;
            super::validate_request_size(image_bytes, maximum, "PutImage")?;
        }
        if !admit(self, cutover.as_deref())? {
            return Ok(PreparedPublication::Skipped);
        }
        self.publication
            .prepare(request, &frame)
            .map_err(publication_error)?;
        self.advance(frame.identity, PublicationEvent::Submit)?;
        let mut cutover = cutover.zip(rectangles).filter(|_| changed);
        if let Some((policy, _)) = cutover.as_mut() {
            policy.block_input();
        }
        // A failure leaves the submitted attempt unsettled. In particular a
        // failed blit/flush can have visible effects. The host fences and exits
        // without reusing this pixmap or automatically attempting another frame.
        upload(dimensions, &frame.payload)?;
        if let Some((policy, _)) = cutover.as_mut() {
            policy.empty_and_release()?;
        }
        blit(dimensions)?;
        self.advance(frame.identity, PublicationEvent::TransportComplete)?;
        let enabling_sequence = cutover
            .map(|(policy, rectangles)| policy.enable(rectangles))
            .transpose()?;
        self.advance(frame.identity, PublicationEvent::Commit)?;
        self.committed = Some(CommittedFrameGeometry {
            identity: frame.identity,
            metadata: frame.metadata,
        });
        Ok(PreparedPublication::Committed(enabling_sequence))
    }

    fn advance(
        &mut self,
        identity: FrameIdentity,
        event: PublicationEvent,
    ) -> Result<(), HostError> {
        self.publication
            .advance(identity, event)
            .map_err(publication_error)
    }

    #[cfg(test)]
    pub(crate) fn last_committed(&self) -> Option<FrameIdentity> {
        self.publication.last_committed()
    }

    #[cfg(test)]
    pub(crate) fn publication_state(
        &self,
        identity: FrameIdentity,
    ) -> Result<PublicationState, PublicationError> {
        self.publication.state(identity)
    }

    #[cfg(test)]
    fn request(&self) -> Result<FrameRequest, HostError> {
        self.frame_request(
            ActionIntent {
                action: Action::Idle,
                facing: Facing::Neutral,
            },
            Duration::ZERO,
            Duration::ZERO,
        )
    }

    #[cfg(test)]
    pub(crate) fn commit_fixture(&mut self, metadata: Option<FrameMetadata>) {
        let request = self.request().unwrap();
        let frame = match metadata {
            Some(metadata) => {
                let size = metadata.extent().size();
                let mut payload = vec![0; size.width as usize * size.height as usize * 4];
                for span in metadata.hit_region().spans() {
                    for x in span.start_x..span.end_x {
                        payload[(span.y as usize * size.width as usize + x as usize) * 4 + 3] = 255;
                    }
                }
                PreparedFrame {
                    identity: request.identity(),
                    metadata,
                    payload,
                }
            }
            None => ProceduralProducer::new(Palette::Lavender)
                .prepare_frame(&request)
                .unwrap(),
        };
        self.publish_with(&request, frame, |_, _| Ok(()), |_| Ok(()))
            .unwrap();
    }
}

fn publication_error(error: PublicationError) -> HostError {
    format!("Renderer publication rejected: {error:?}").into()
}

fn validate_bgra8(
    (width, height): (u16, u16),
    frame: &PreparedFrame<Vec<u8>>,
) -> Result<(), HostError> {
    let width = usize::from(width);
    let expected_bytes = width
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("BGRA8 payload size exceeds host limits")?;
    if frame.payload.len() != expected_bytes {
        return Err("BGRA8 payload has an unexpected byte length".into());
    }

    // Canonical spans yield covered pixel indices in the same order as the
    // tightly packed rows, including disjoint spans and transparent padding.
    let mut covered = frame
        .metadata
        .hit_region()
        .spans()
        .iter()
        .flat_map(|span| {
            let row_start = span.y as usize * width;
            (span.start_x..span.end_x).map(move |x| row_start + x as usize)
        })
        .peekable();
    let (pixels, _) = frame.payload.as_chunks::<4>();
    for (index, pixel) in pixels.iter().enumerate() {
        let alpha = pixel[3];
        if pixel[..3].iter().any(|&channel| channel > alpha) {
            return Err("BGRA8 payload contains a non-premultiplied pixel".into());
        }
        if (alpha != 0) != covered.next_if_eq(&index).is_some() {
            return Err("BGRA8 alpha does not match frame hit coverage".into());
        }
    }
    Ok(())
}

pub(crate) fn paint_committed_with(
    geometry: &FrameGeometry,
    blit: impl FnOnce((u16, u16)) -> Result<(), HostError>,
) -> Result<(), HostError> {
    blit(native_dimensions(
        geometry.require_committed()?.metadata.extent(),
    )?)
}

pub(crate) fn checked_blit_with(
    blit: impl FnOnce() -> Result<(), HostError>,
    flush: impl FnOnce() -> Result<(), HostError>,
) -> Result<(), HostError> {
    blit()?;
    flush()
}

pub struct Renderer {
    pub gc_resource: OwnedResource,
    pub pixmap_resource: OwnedResource,

    pub pixmap: Pixmap,
    pub gc: Gcontext,
}

impl Renderer {
    pub fn new(
        conn: &impl Connection,
        window: Window,
        extent: ClientExtent,
    ) -> Result<Self, HostError> {
        let (width, height) = native_dimensions(extent)?;

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
        Ok(Self {
            pixmap_resource,
            gc_resource: OwnedResource::default(),
            pixmap,
            gc,
        })
    }

    #[cfg(test)]
    pub fn initial_extent() -> ClientExtent {
        ProceduralProducer::new(Palette::Lavender)
            .capabilities()
            .initial_extent()
    }

    /// Blit the committed Pixmap on Expose without creating a new frame.
    pub fn paint(
        &self,
        conn: &impl Connection,
        window: Window,
        geometry: &FrameGeometry,
    ) -> Result<(), HostError> {
        paint_committed_with(geometry, |dimensions| self.blit(conn, window, dimensions))
    }

    fn blit(
        &self,
        conn: &impl Connection,
        window: Window,
        (width, height): (u16, u16),
    ) -> Result<(), HostError> {
        checked_blit_with(
            || {
                conn.copy_area(self.pixmap, window, self.gc, 0, 0, 0, 0, width, height)?
                    .check()?;
                Ok(())
            },
            || Ok(conn.flush()?),
        )
    }

    pub fn present(
        &self,
        conn: &impl Connection,
        window: Window,
        geometry: &mut FrameGeometry,
        request: &FrameRequest,
        frame: PreparedFrame<Vec<u8>>,
        cutover: Option<&mut dyn CoverageCutover>,
    ) -> Result<Option<u64>, HostError> {
        geometry.publish_with_cutover(
            request,
            frame,
            |dimensions, payload| self.upload(conn, dimensions, payload),
            |dimensions| self.blit(conn, window, dimensions),
            cutover,
        )
    }

    #[cfg(feature = "managed-supervisor-fixture")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn present_conditionally(
        &self,
        conn: &impl Connection,
        window: Window,
        geometry: &mut FrameGeometry,
        request: &FrameRequest,
        frame: PreparedFrame<Vec<u8>>,
        cutover: &mut dyn CoverageCutover,
        admit: impl FnOnce(&FrameGeometry, Option<&dyn CoverageCutover>) -> Result<bool, HostError>,
    ) -> Result<PreparedPublication, HostError> {
        geometry.publish_with_admission(
            request,
            frame,
            |dimensions, payload| self.upload(conn, dimensions, payload),
            |dimensions| self.blit(conn, window, dimensions),
            Some(cutover),
            admit,
        )
    }

    fn upload(
        &self,
        conn: &impl Connection,
        (width, height): (u16, u16),
        payload: &[u8],
    ) -> Result<(), HostError> {
        conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.pixmap,
            self.gc,
            width,
            height,
            0,
            0,
            0,
            32,
            payload,
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
    use crate::presentation::{HitRegion, RowSpan, Size};
    use std::cell::RefCell;

    #[test]
    fn renderer_theme_starts_lavender_and_alternates_adapter_palettes() {
        let initial = Palette::default();
        let clicked = initial.toggled();
        let clicked_again = clicked.toggled();
        assert_eq!(
            (initial, clicked, clicked_again),
            (Palette::Lavender, Palette::Coral, initial)
        );
        let mut producer = ProceduralProducer::new(initial);
        assert_eq!(
            [
                initial,
                producer.toggle_palette(),
                producer.toggle_palette()
            ],
            [Palette::Lavender, Palette::Coral, Palette::Lavender]
        );
    }

    #[test]
    fn publish_commits_the_requested_metadata_after_checked_upload_and_blit() {
        let extent = Renderer::initial_extent();
        let mut geometry = FrameGeometry::new(extent);
        assert_eq!(geometry.initial_extent(), extent);
        assert!(geometry.committed().is_none());
        let trace = RefCell::new(Vec::new());
        for (sequence, palette) in [Palette::Lavender, Palette::Coral].into_iter().enumerate() {
            let request = geometry.request().unwrap();
            let frame = ProceduralProducer::new(palette)
                .prepare_frame(&request)
                .unwrap();
            let metadata = frame.metadata.clone();
            let identity = FrameIdentity {
                generation: 0,
                sequence: sequence as u64,
            };
            assert_eq!(request.identity(), identity);
            assert_ne!(
                geometry.committed().map(|frame| frame.identity),
                Some(identity)
            );
            geometry
                .publish_with(
                    &request,
                    frame,
                    |dimensions, pixels| {
                        trace.borrow_mut().push("checked upload");
                        assert_eq!(dimensions, (160, 160));
                        assert_eq!(pixels.len(), 160 * 160 * 4);
                        Ok(())
                    },
                    |dimensions| {
                        trace.borrow_mut().push("checked blit");
                        assert_eq!(dimensions, (160, 160));
                        Ok(())
                    },
                )
                .unwrap();
            let committed = geometry.require_committed().unwrap();
            assert_eq!(committed.identity, identity);
            assert_eq!(committed.metadata, metadata);
        }
        assert_eq!(
            trace.into_inner(),
            [
                "checked upload",
                "checked blit",
                "checked upload",
                "checked blit"
            ]
        );
    }

    #[test]
    fn stale_mismatched_and_coverage_changing_candidates_do_not_replace_commit_or_send() {
        let mut geometry = FrameGeometry::new(Renderer::initial_extent());
        let initial_request = geometry.request().unwrap();
        geometry.commit_fixture(None);
        let mut producer = ProceduralProducer::new(Palette::Coral);
        let stale = producer.prepare_frame(&initial_request).unwrap();
        let request = geometry.request().unwrap();
        let mut wrong_identity = producer.prepare_frame(&request).unwrap();
        wrong_identity.identity = INITIAL_IDENTITY;
        let extent = request.extent();
        let resized_request = FrameRequest::new(
            request.identity(),
            request.intent(),
            ClientExtent::new(Size::new(73, 49)).unwrap(),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        let mut expanded_region = producer.prepare_frame(&request).unwrap();
        let spans = std::iter::once(RowSpan {
            y: 0,
            start_x: 0,
            end_x: 1,
        })
        .chain(
            expanded_region
                .metadata
                .hit_region()
                .spans()
                .iter()
                .copied(),
        );
        expanded_region.metadata =
            FrameMetadata::new(extent, None, None, HitRegion::new(extent, spans).unwrap()).unwrap();
        let mut changed_region = producer.prepare_frame(&request).unwrap();
        changed_region.metadata =
            FrameMetadata::new(extent, None, None, HitRegion::new(extent, []).unwrap()).unwrap();
        let previous = geometry.require_committed().unwrap().metadata.clone();
        for (request, frame, diagnostic) in [
            (initial_request, stale, "Stale"),
            (request, wrong_identity, "IdentityMismatch"),
            (
                resized_request,
                producer.prepare_frame(&request).unwrap(),
                "extent changes are unsupported",
            ),
            (request, expanded_region, "coverage changes are unsupported"),
            (request, changed_region, "coverage changes are unsupported"),
        ] {
            let error = geometry
                .publish_with(
                    &request,
                    frame,
                    |_, _| panic!("rejected frame must not upload"),
                    |_| panic!("rejected frame must not blit"),
                )
                .unwrap_err();
            assert!(error.to_string().contains(diagnostic), "{error}");
            let current = geometry.require_committed().unwrap();
            assert_eq!(current.identity, INITIAL_IDENTITY);
            assert_eq!(current.metadata, previous);
        }
    }

    #[test]
    fn failed_upload_or_blit_never_commits_or_retries_and_retains_history_only() {
        for (initially_committed, fails_at_blit) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let mut geometry = FrameGeometry::new(Renderer::initial_extent());
            if initially_committed {
                geometry.commit_fixture(None);
            }
            let request = geometry.request().unwrap();
            let mut producer = ProceduralProducer::new(Palette::Coral);
            let frame = producer.prepare_frame(&request).unwrap();
            let error = geometry
                .publish_with(
                    &request,
                    frame,
                    |_, _| {
                        if fails_at_blit {
                            Ok(())
                        } else {
                            Err("upload failure".into())
                        }
                    },
                    |_| {
                        assert!(fails_at_blit, "failed upload must prevent blit");
                        Err("possibly visible blit failure".into())
                    },
                )
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                if fails_at_blit {
                    "possibly visible blit failure"
                } else {
                    "upload failure"
                }
            );
            assert!(geometry.committed().is_none());
            assert!(geometry.publication_uncertain());
            assert_eq!(
                geometry.committed.as_ref().map(|prior| prior.identity),
                initially_committed.then_some(INITIAL_IDENTITY)
            );
            assert!(geometry.request().is_err());
            let stale_frame = producer.prepare_frame(&request).unwrap();
            geometry.invalidate();
            assert!(geometry
                .publish_with(
                    &request,
                    stale_frame,
                    |_, _| panic!("no retry"),
                    |_| panic!("no retry")
                )
                .is_err());
            assert!(geometry.committed().is_none());
        }
    }

    #[test]
    fn invalidated_provisional_or_committed_target_cannot_issue_a_frame() {
        for committed in [false, true] {
            let mut geometry = FrameGeometry::new(Renderer::initial_extent());
            if committed {
                geometry.commit_fixture(None);
            }
            geometry.invalidate();
            assert!(geometry.request().is_err());
            assert!(geometry.committed().is_none());
            assert_eq!(geometry.initial_extent(), Renderer::initial_extent());
        }
    }

    #[test]
    fn malformed_pixels_or_metadata_never_reach_native_submission() {
        let extent = Renderer::initial_extent();
        let mut geometry = FrameGeometry::new(extent);
        geometry.commit_fixture(None);
        let request = geometry.request().unwrap();
        let mut producer = ProceduralProducer::new(Palette::Coral);
        for (defect, expected) in [
            ("short", "unexpected byte length"),
            ("long", "unexpected byte length"),
            ("straight alpha", "non-premultiplied"),
            ("covered transparency", "alpha does not match"),
            ("uncovered alpha", "alpha does not match"),
            ("wrong extent", "ExtentMismatch"),
        ] {
            let mut frame = producer.prepare_frame(&request).unwrap();
            let center = (80 * 160 + 80) * 4;
            match defect {
                "short" => frame.payload.truncate(frame.payload.len() - 1),
                "long" => frame.payload.push(0),
                "straight alpha" => frame.payload[0] = 1,
                "covered transparency" => frame.payload[center..center + 4].fill(0),
                "uncovered alpha" => frame.payload[3] = 1,
                "wrong extent" => {
                    let wrong_extent = ClientExtent::new(Size::new(159, 160)).unwrap();
                    frame.metadata = FrameMetadata::new(
                        wrong_extent,
                        None,
                        None,
                        HitRegion::new(wrong_extent, []).unwrap(),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let error = geometry
                .publish_with(
                    &request,
                    frame,
                    |_, _| panic!("invalid frame must not upload"),
                    |_| panic!("invalid frame must not blit"),
                )
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{defect}: {error}");
            assert_eq!(
                geometry.publication.state(request.identity()),
                Err(PublicationError::IdentityMismatch)
            );
            assert_eq!(
                geometry.require_committed().unwrap().identity,
                INITIAL_IDENTITY
            );
        }
    }

    #[test]
    fn native_dimension_and_shape_limits_reject_first_frame_before_submission() {
        for (width, expected) in [
            (u32::from(u16::MAX) + 1, "X11"),
            (i16::MAX as u32 + 1, "input region right edge"),
        ] {
            let extent = ClientExtent::new(Size::new(width, 1)).unwrap();
            let mut geometry = FrameGeometry::new(extent);
            let request = geometry.request().unwrap();
            let region = HitRegion::new(
                extent,
                [RowSpan {
                    y: 0,
                    start_x: width as i32 - 1,
                    end_x: width as i32,
                }],
            )
            .unwrap();
            let mut payload = vec![0; width as usize * 4];
            *payload.last_mut().unwrap() = 255;
            let frame = PreparedFrame {
                identity: request.identity(),
                metadata: FrameMetadata::new(extent, None, None, region).unwrap(),
                payload,
            };
            let error = geometry
                .publish_with(
                    &request,
                    frame,
                    |_, _| panic!("unrepresentable frame must not upload"),
                    |_| panic!("unrepresentable frame must not blit"),
                )
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert!(geometry.committed().is_none());
        }
    }

    #[test]
    fn host_frame_requests_stop_at_sequence_and_generation_exhaustion() {
        let mut geometry = FrameGeometry::new(Renderer::initial_extent());
        for generation in [0, u64::MAX] {
            geometry.publication = Publication::new(generation);
            let identity = FrameIdentity {
                generation,
                sequence: u64::MAX,
            };
            let request = FrameRequest::new(
                identity,
                ActionIntent {
                    action: Action::Idle,
                    facing: Facing::Neutral,
                },
                geometry.initial_extent(),
                Duration::ZERO,
                Duration::ZERO,
            )
            .unwrap();
            let frame = ProceduralProducer::new(Palette::Lavender)
                .prepare_frame(&request)
                .unwrap();
            geometry
                .publish_with(&request, frame, |_, _| Ok(()), |_| Ok(()))
                .unwrap();
            assert_eq!(geometry.require_committed().unwrap().identity, identity);
            assert!(geometry
                .request()
                .unwrap_err()
                .to_string()
                .contains("sequence exhausted"));
            geometry.invalidate();
            assert!(geometry
                .request()
                .unwrap_err()
                .to_string()
                .contains("invalidated"));
            assert!(geometry.committed().is_none());
            assert_eq!(
                geometry.publication.state(identity),
                Ok(PublicationState::Stale)
            );
        }
        assert_eq!(
            geometry.publication.invalidate(),
            Err(PublicationError::GenerationExhausted)
        );
    }
}
