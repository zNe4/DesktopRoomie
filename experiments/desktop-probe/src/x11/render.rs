use super::{
    native_dimensions,
    resource::{cleanup_all, OwnedResource},
    shape::input_rectangles,
};
use crate::presentation::procedural::{Palette, ProceduralProducer};
use crate::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameMetadata, FrameProducer,
    FrameRequest, PreparedFrame, Publication, PublicationEvent, PublicationState,
};
use crate::HostError;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, CreateGCAux, Gcontext, ImageFormat, Pixmap, Window,
};

const INITIAL_IDENTITY: FrameIdentity = FrameIdentity {
    generation: 0,
    sequence: 0,
};

pub(crate) struct CommittedFrameGeometry {
    pub identity: FrameIdentity,
    pub metadata: FrameMetadata,
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

    fn request(&self) -> Result<FrameRequest, HostError> {
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
        FrameRequest::new(
            identity,
            ActionIntent {
                action: Action::Idle,
                facing: Facing::Neutral,
            },
            extent,
            Duration::ZERO,
            Duration::ZERO,
        )
        .map_err(|error| format!("Invalid renderer frame request: {error:?}").into())
    }

    fn publish_with(
        &mut self,
        request: &FrameRequest,
        frame: PreparedFrame<Vec<u8>>,
        upload: impl FnOnce((u16, u16), &[u8]) -> Result<(), HostError>,
        blit: impl FnOnce((u16, u16)) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        if request.extent() != self.initial_extent {
            return Err("Runtime frame extent changes are unsupported".into());
        }
        let dimensions = native_dimensions(frame.metadata.extent())?;
        if let Some(committed) = self.committed() {
            if committed.metadata.hit_region() != frame.metadata.hit_region() {
                return Err("Runtime frame coverage changes are unsupported".into());
            }
        } else {
            // Validate the first coverage before it can ever become authoritative.
            input_rectangles(frame.metadata.hit_region())?;
        }
        self.publication
            .prepare(request, &frame)
            .map_err(publication_error)?;
        self.advance(frame.identity, PublicationEvent::Submit)?;
        // A failure leaves the submitted attempt unsettled. In particular a
        // failed blit/flush can have visible effects. The host fences and exits
        // without reusing this pixmap or automatically attempting another frame.
        upload(dimensions, &frame.payload)?;
        blit(dimensions)?;
        self.advance(frame.identity, PublicationEvent::TransportComplete)?;
        self.advance(frame.identity, PublicationEvent::Commit)?;
        self.committed = Some(CommittedFrameGeometry {
            identity: frame.identity,
            metadata: frame.metadata,
        });
        Ok(())
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
    pub(crate) fn commit_fixture(&mut self, metadata: Option<FrameMetadata>) {
        let request = self.request().unwrap();
        let mut frame = ProceduralProducer::new(Palette::Lavender);
        let frame = match metadata {
            Some(metadata) => PreparedFrame {
                identity: request.identity(),
                metadata,
                payload: Vec::new(),
            },
            None => frame.prepare_frame(&request).unwrap(),
        };
        self.publish_with(&request, frame, |_, _| Ok(()), |_| Ok(()))
            .unwrap();
    }
}

fn publication_error(error: crate::presentation::PublicationError) -> HostError {
    format!("Renderer publication rejected: {error:?}").into()
}

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
    theme: ColorTheme,
}

impl Renderer {
    pub fn new(
        conn: &impl Connection,
        window: Window,
        extent: ClientExtent,
    ) -> Result<Self, HostError> {
        let (width, height) = native_dimensions(extent)?;
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
        Ok(Self {
            pixmap_resource,
            gc_resource: OwnedResource::default(),
            pixmap,
            gc,
            theme,
        })
    }

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
        self.blit(
            conn,
            window,
            native_dimensions(geometry.require_committed()?.metadata.extent())?,
        )
    }

    fn blit(
        &self,
        conn: &impl Connection,
        window: Window,
        (width, height): (u16, u16),
    ) -> Result<(), HostError> {
        conn.copy_area(self.pixmap, window, self.gc, 0, 0, 0, 0, width, height)?
            .check()?;
        conn.flush()?;
        Ok(())
    }

    pub fn present_initial(
        &self,
        conn: &impl Connection,
        window: Window,
        geometry: &mut FrameGeometry,
    ) -> Result<(), HostError> {
        self.present(conn, window, geometry, self.theme)
    }

    /// Toggles the body color palette after checked presentation succeeds.
    pub fn toggle_color(
        &mut self,
        conn: &impl Connection,
        window: Window,
        geometry: &mut FrameGeometry,
    ) -> Result<ColorTheme, HostError> {
        let theme = self.theme.toggled();
        self.present(conn, window, geometry, theme)?;
        self.theme = theme;
        Ok(theme)
    }

    fn present(
        &self,
        conn: &impl Connection,
        window: Window,
        geometry: &mut FrameGeometry,
        theme: ColorTheme,
    ) -> Result<(), HostError> {
        let request = geometry.request()?;
        let frame = ProceduralProducer::new(theme.palette()).prepare_frame(&request)?;
        geometry.publish_with(
            &request,
            frame,
            |(width, height), payload| {
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
            },
            |dimensions| self.blit(conn, window, dimensions),
        )
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
}
