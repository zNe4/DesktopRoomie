use std::time::Duration;

use desktop_probe::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameMetadata, FrameProducer,
    FrameRequest, HitRegion, Point, PreparedFrame, PresentationCapabilities, Publication,
    PublicationError, PublicationEvent as Event, PublicationFailure, PublicationState as State,
    Rect, RowSpan, Size, ValidationError,
};

const IDLE: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};
const WALK: ActionIntent = ActionIntent {
    action: Action::Walk,
    facing: Facing::FrontRight,
};
const FIRST: FrameIdentity = FrameIdentity {
    generation: 4,
    sequence: 10,
};

fn extent() -> ClientExtent {
    ClientExtent::new(Size::new(8, 6)).unwrap()
}

fn span(y: i32, start_x: i32, end_x: i32) -> RowSpan {
    RowSpan { y, start_x, end_x }
}

fn empty_metadata(extent: ClientExtent) -> FrameMetadata {
    let region = HitRegion::new(extent, []).unwrap();
    FrameMetadata::new(extent, None, None, region).unwrap()
}

fn request(identity: FrameIdentity) -> FrameRequest {
    FrameRequest::new(identity, IDLE, extent(), Duration::ZERO, Duration::ZERO).unwrap()
}

fn prepare(publication: &mut Publication, identity: FrameIdentity) -> Result<(), PublicationError> {
    publication.prepare(
        &request(identity),
        &PreparedFrame {
            identity,
            metadata: empty_metadata(extent()),
            payload: (),
        },
    )
}

fn admit(publication: &mut Publication, identity: FrameIdentity) {
    prepare(publication, identity).unwrap();
}

fn commit_prepared(publication: &mut Publication, identity: FrameIdentity) {
    for event in [Event::Submit, Event::TransportComplete, Event::Commit] {
        publication.advance(identity, event).unwrap();
    }
}

#[test]
fn extent_rejects_zero_and_controls_coordinate_limits() {
    assert_eq!(extent().size(), Size::new(8, 6));
    for size in [Size::new(0, 6), Size::new(8, 0), Size::default()] {
        assert_eq!(ClientExtent::new(size), Err(ValidationError::EmptyExtent));
    }
    let limit = i32::MAX as u32;
    for size in [
        Size::new(limit + 1, 1),
        Size::new(1, limit + 1),
        Size::new(u32::MAX, 1),
    ] {
        assert_eq!(
            ClientExtent::new(size),
            Err(ValidationError::CoordinateOverflow)
        );
    }
    let max = ClientExtent::new(Size::new(limit, limit)).unwrap();
    assert!(max.contains(Point::new(i32::MAX - 1, i32::MAX - 1)));
    assert!(!max.contains(Point::new(i32::MAX, 0)));
    assert!(!max.contains(Point::new(0, i32::MAX)));
}

#[test]
fn canonical_hit_region_merges_adjacency_and_keeps_holes_and_row_boundaries() {
    let region = HitRegion::new(
        extent(),
        [
            span(0, 0, 2),
            span(0, 2, 4),
            span(0, 6, 8),
            span(2, 0, 4),
            span(5, 7, 8),
        ],
    )
    .unwrap();
    assert_eq!(region.extent(), extent());
    assert_eq!(
        region.spans(),
        &[span(0, 0, 4), span(0, 6, 8), span(2, 0, 4), span(5, 7, 8)]
    );
    for point in [(0, 0), (2, 0), (3, 0), (6, 0), (7, 0), (0, 2), (7, 5)] {
        assert!(region.contains(Point::new(point.0, point.1)), "{point:?}");
    }
    for point in [
        (-1, 0),
        (0, -1),
        (4, 0),
        (5, 0),
        (8, 0),
        (0, 1),
        (4, 2),
        (7, 6),
    ] {
        assert!(!region.contains(Point::new(point.0, point.1)), "{point:?}");
    }
}

#[test]
fn empty_region_has_no_interactive_pixels() {
    let region = HitRegion::new(extent(), []).unwrap();
    assert!(region.spans().is_empty());
    assert!(!region.contains(Point::new(0, 0)));
}

#[test]
fn hit_region_rejects_unsorted_overlapping_and_empty_spans() {
    for spans in [
        [span(1, 0, 2), span(0, 0, 2)],
        [span(0, 3, 5), span(0, 0, 2)],
    ] {
        assert_eq!(
            HitRegion::new(extent(), spans),
            Err(ValidationError::UnsortedSpans)
        );
    }
    for spans in [
        [span(0, 1, 3), span(0, 2, 4)],
        [span(0, 1, 3), span(0, 1, 3)],
    ] {
        assert_eq!(
            HitRegion::new(extent(), spans),
            Err(ValidationError::OverlappingSpans)
        );
    }
    for row in [span(0, 3, 3), span(0, 4, 3)] {
        assert_eq!(
            HitRegion::new(extent(), [row]),
            Err(ValidationError::EmptySpan)
        );
    }
}

#[test]
fn hit_region_checks_client_edges_and_signed_coordinate_extremes() {
    for row in [
        span(-1, 0, 1),
        span(6, 0, 1),
        span(0, -1, 1),
        span(0, 7, 9),
        span(0, 8, 9),
        span(i32::MAX, i32::MIN, i32::MAX),
    ] {
        assert_eq!(
            HitRegion::new(extent(), [row]),
            Err(ValidationError::SpanOutOfBounds)
        );
    }
    let edge = i32::MAX;
    let max = ClientExtent::new(Size::new(edge as u32, edge as u32)).unwrap();
    let region = HitRegion::new(max, [span(edge - 1, edge - 1, edge)]).unwrap();
    assert!(region.contains(Point::new(edge - 1, edge - 1)));
    assert!(!region.contains(Point::new(edge, edge - 1)));
    assert!(!region.contains(Point::new(edge - 1, edge)));
}

#[test]
fn metadata_accepts_independent_optional_anchor_and_bounds() {
    let region = HitRegion::new(extent(), [span(1, 5, 8)]).unwrap();
    let anchor = Point::new(0, 5);
    let bounds = Rect::new(5, 1, 3, 1);
    for (visible_bounds, anchor) in [
        (Some(bounds), Some(anchor)),
        (Some(bounds), None),
        (None, Some(anchor)),
        (None, None),
    ] {
        let metadata =
            FrameMetadata::new(extent(), visible_bounds, anchor, region.clone()).unwrap();
        assert_eq!(metadata.extent(), extent());
        assert_eq!(metadata.visible_bounds(), visible_bounds);
        assert_eq!(metadata.anchor(), anchor);
        assert_eq!(metadata.hit_region(), &region);
    }
}

#[test]
fn metadata_rejects_invalid_anchor_and_bounds() {
    let region = HitRegion::new(extent(), []).unwrap();
    for anchor in [
        Point::new(-1, 0),
        Point::new(0, -1),
        Point::new(8, 0),
        Point::new(0, 6),
    ] {
        assert_eq!(
            FrameMetadata::new(extent(), None, Some(anchor), region.clone()),
            Err(ValidationError::AnchorOutOfBounds)
        );
    }
    for bounds in [
        Rect::new(0, 0, 0, 1),
        Rect::new(0, 0, 1, 0),
        Rect::new(-1, 0, 2, 1),
        Rect::new(0, -1, 1, 2),
        Rect::new(7, 0, 2, 1),
        Rect::new(0, 5, 1, 2),
        Rect::new(8, 0, 1, 1),
    ] {
        assert_eq!(
            FrameMetadata::new(extent(), Some(bounds), None, region.clone()),
            Err(ValidationError::InvalidVisibleBounds)
        );
    }
    for bounds in [
        Rect::new(1, 0, i32::MAX as u32, 1),
        Rect::new(0, 1, 1, u32::MAX),
    ] {
        assert_eq!(
            FrameMetadata::new(extent(), Some(bounds), None, region.clone()),
            Err(ValidationError::CoordinateOverflow)
        );
    }
}

#[test]
fn metadata_requires_matching_region_extent_and_contained_coverage() {
    let smaller = ClientExtent::new(Size::new(7, 6)).unwrap();
    let region = HitRegion::new(extent(), [span(2, 2, 5)]).unwrap();
    assert_eq!(
        FrameMetadata::new(smaller, None, None, region.clone()),
        Err(ValidationError::RegionExtentMismatch)
    );
    for bounds in [
        Rect::new(3, 2, 2, 1),
        Rect::new(2, 2, 2, 1),
        Rect::new(2, 3, 3, 1),
        Rect::new(2, 1, 3, 1),
    ] {
        assert_eq!(
            FrameMetadata::new(extent(), Some(bounds), None, region.clone()),
            Err(ValidationError::CoverageOutsideVisibleBounds)
        );
    }
}

#[test]
fn capabilities_match_exact_action_and_facing_pairs() {
    let period = Duration::from_millis(16);
    let capabilities =
        PresentationCapabilities::new(extent(), vec![IDLE, WALK], Some(period)).unwrap();
    assert_eq!(capabilities.initial_extent(), extent());
    assert_eq!(capabilities.supported_intents(), &[IDLE, WALK]);
    assert_eq!(capabilities.frame_period(), Some(period));
    assert!(capabilities.supports(IDLE));
    assert!(capabilities.supports(WALK));
    assert!(!capabilities.supports(ActionIntent {
        action: Action::Idle,
        ..WALK
    }));
    for facing in [
        Facing::Neutral,
        Facing::FrontLeft,
        Facing::BackRight,
        Facing::BackLeft,
    ] {
        assert!(!capabilities.supports(ActionIntent { facing, ..WALK }));
    }
    assert_eq!(
        PresentationCapabilities::new(extent(), vec![IDLE], Some(Duration::ZERO)),
        Err(ValidationError::ZeroFramePeriod)
    );
}

#[test]
fn frame_request_preserves_active_time_and_checks_delta() {
    let elapsed = Duration::from_secs(5);
    let delta = Duration::from_millis(20);
    let frame = FrameRequest::new(FIRST, WALK, extent(), elapsed, delta).unwrap();
    assert_eq!(frame.identity(), FIRST);
    assert_eq!(frame.intent(), WALK);
    assert_eq!(frame.extent(), extent());
    assert_eq!(frame.active_elapsed(), elapsed);
    assert_eq!(frame.active_delta(), delta);
    assert_eq!(
        FrameRequest::new(FIRST, WALK, extent(), Duration::ZERO, delta),
        Err(ValidationError::DeltaExceedsElapsed)
    );
}

#[test]
fn sequence_exhaustion_cannot_wrap() {
    let penultimate = FrameIdentity {
        sequence: u64::MAX - 1,
        ..FIRST
    };
    let last = FrameIdentity {
        sequence: u64::MAX,
        ..FIRST
    };
    assert_eq!(penultimate.checked_next(), Some(last));
    assert_eq!(last.checked_next(), None);
    let mut publication = Publication::new(last.generation);
    admit(&mut publication, last);
    assert_eq!(
        publication.prepare(
            &request(FIRST),
            &PreparedFrame {
                identity: FIRST,
                metadata: empty_metadata(extent()),
                payload: (),
            }
        ),
        Err(PublicationError::Stale)
    );
}

#[test]
fn publication_rejects_mismatched_prepared_frame_identity_or_extent() {
    let mut publication = Publication::new(FIRST.generation);
    let mut frame = PreparedFrame {
        identity: FIRST.checked_next().unwrap(),
        metadata: empty_metadata(extent()),
        payload: (),
    };
    assert_eq!(
        publication.prepare(&request(FIRST), &frame),
        Err(PublicationError::IdentityMismatch)
    );
    frame.identity = FIRST;
    frame.metadata = empty_metadata(ClientExtent::new(Size::new(3, 3)).unwrap());
    assert_eq!(
        publication.prepare(&request(FIRST), &frame),
        Err(PublicationError::ExtentMismatch)
    );
    assert_eq!(
        publication.state(FIRST),
        Err(PublicationError::IdentityMismatch)
    );
    admit(&mut publication, FIRST);
}

#[test]
fn publication_requires_all_steps_and_rejects_duplicate_commit() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    assert_eq!(publication.state(FIRST), Ok(State::Prepared));
    assert_eq!(
        publication.advance(FIRST, Event::TransportComplete),
        Err(PublicationError::InvalidTransition)
    );
    assert_eq!(
        publication.advance(FIRST, Event::Commit),
        Err(PublicationError::InvalidTransition)
    );
    publication.advance(FIRST, Event::Submit).unwrap();
    assert_eq!(publication.state(FIRST), Ok(State::Submitted));
    assert_eq!(
        publication.advance(FIRST, Event::Submit),
        Err(PublicationError::InvalidTransition)
    );
    assert_eq!(
        publication.advance(FIRST, Event::Commit),
        Err(PublicationError::InvalidTransition)
    );
    publication
        .advance(FIRST, Event::TransportComplete)
        .unwrap();
    assert_eq!(publication.state(FIRST), Ok(State::TransportCompleted));
    assert_eq!(
        publication.advance(FIRST, Event::TransportComplete),
        Err(PublicationError::InvalidTransition)
    );
    publication.advance(FIRST, Event::Commit).unwrap();
    assert_eq!(publication.state(FIRST), Ok(State::HostCommitted));
    assert_eq!(
        publication.advance(FIRST, Event::Commit),
        Err(PublicationError::InvalidTransition)
    );
    assert_eq!(publication.state(FIRST), Ok(State::HostCommitted));
}

#[test]
fn newer_admission_makes_older_completions_and_commits_obsolete() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    let newer = FIRST.checked_next().unwrap();
    assert_eq!(
        publication.advance(newer, Event::TransportComplete),
        Err(PublicationError::IdentityMismatch)
    );
    admit(&mut publication, newer);
    assert_eq!(publication.state(FIRST), Ok(State::Stale));
    assert_eq!(
        publication.advance(FIRST, Event::TransportComplete),
        Err(PublicationError::Stale)
    );
    for identity in [FIRST, newer] {
        let frame = PreparedFrame {
            identity,
            metadata: empty_metadata(extent()),
            payload: (),
        };
        assert_eq!(
            publication.prepare(&request(identity), &frame),
            Err(PublicationError::Stale)
        );
    }
    publication.advance(newer, Event::Submit).unwrap();
    publication
        .advance(newer, Event::TransportComplete)
        .unwrap();
    assert_eq!(
        publication.advance(FIRST, Event::Commit),
        Err(PublicationError::Stale)
    );
    assert_eq!(publication.state(newer), Ok(State::TransportCompleted));
    publication.advance(newer, Event::Commit).unwrap();
}

#[test]
fn generation_invalidation_rejects_old_evidence_even_with_reused_sequence() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    publication.advance(FIRST, Event::Submit).unwrap();
    publication
        .advance(FIRST, Event::TransportComplete)
        .unwrap();
    let replacement = FrameIdentity {
        generation: FIRST.generation + 1,
        ..FIRST
    };
    assert_eq!(
        publication.state(replacement),
        Err(PublicationError::IdentityMismatch)
    );
    assert_eq!(publication.invalidate(), Ok(replacement.generation));
    assert_eq!(publication.state(FIRST), Ok(State::Stale));
    assert_eq!(
        publication.advance(FIRST, Event::Commit),
        Err(PublicationError::Stale)
    );
    publication
        .advance(
            FIRST,
            Event::Fail(PublicationFailure::BeforeVisibleMutation),
        )
        .unwrap();
    admit(&mut publication, replacement);
    assert_eq!(
        publication.advance(FIRST, Event::Fail(PublicationFailure::PossiblyVisible)),
        Err(PublicationError::Stale)
    );
    let old_frame = PreparedFrame {
        identity: FIRST,
        metadata: empty_metadata(extent()),
        payload: (),
    };
    assert_eq!(
        publication.prepare(&request(FIRST), &old_frame),
        Err(PublicationError::Stale)
    );
    assert_eq!(publication.state(replacement), Ok(State::Prepared));
}

#[test]
fn generation_exhaustion_closes_target_to_pending_and_new_frames() {
    let last = FrameIdentity {
        generation: u64::MAX,
        ..FIRST
    };
    let mut publication = Publication::new(last.generation);
    admit(&mut publication, last);
    assert_eq!(
        publication.invalidate(),
        Err(PublicationError::GenerationExhausted)
    );
    assert_eq!(publication.state(last), Ok(State::Stale));
    let wrapped = FrameIdentity {
        generation: 0,
        ..last
    };
    assert_eq!(
        publication.advance(wrapped, Event::Submit),
        Err(PublicationError::Stale)
    );
    assert_eq!(
        publication.advance(last, Event::Commit),
        Err(PublicationError::Stale)
    );
}

#[test]
fn publication_distinguishes_failures_before_visible_mutation_and_uncertain_effects() {
    let early = PublicationFailure::BeforeVisibleMutation;
    let uncertain = PublicationFailure::PossiblyVisible;
    for (prior_events, failure) in [
        (&[][..], early),
        (&[Event::Submit][..], early),
        (&[Event::Submit][..], uncertain),
        (&[Event::Submit, Event::TransportComplete][..], uncertain),
    ] {
        let mut publication = Publication::new(FIRST.generation);
        admit(&mut publication, FIRST);
        for event in prior_events {
            publication.advance(FIRST, *event).unwrap();
        }
        publication.advance(FIRST, Event::Fail(failure)).unwrap();
        assert_eq!(publication.state(FIRST), Ok(State::Failed(failure)));
        assert_eq!(
            publication.advance(FIRST, Event::Commit),
            Err(PublicationError::InvalidTransition)
        );
        let same_frame = PreparedFrame {
            identity: FIRST,
            metadata: empty_metadata(extent()),
            payload: (),
        };
        assert_eq!(
            publication.prepare(&request(FIRST), &same_frame),
            Err(PublicationError::Stale)
        );
    }
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    assert_eq!(
        publication.advance(FIRST, Event::Fail(uncertain)),
        Err(PublicationError::InvalidTransition)
    );
    assert_eq!(publication.state(FIRST), Ok(State::Prepared));
}

#[test]
fn failure_before_visible_mutation_preserves_the_previous_commit() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    commit_prepared(&mut publication, FIRST);
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), Some(FIRST));

    let second = FIRST.checked_next().unwrap();
    admit(&mut publication, second);
    assert_eq!(publication.state(second), Ok(State::Prepared));
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), Some(FIRST));

    let failure = PublicationFailure::BeforeVisibleMutation;
    publication.advance(second, Event::Fail(failure)).unwrap();
    assert_eq!(publication.state(second), Ok(State::Failed(failure)));
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), Some(FIRST));
    assert!(!publication.visibility_uncertain());
}

#[test]
fn committing_a_new_candidate_replaces_the_previous_commit() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    commit_prepared(&mut publication, FIRST);

    let second = FIRST.checked_next().unwrap();
    admit(&mut publication, second);
    assert_eq!(publication.last_committed(), Some(FIRST));
    publication.advance(second, Event::Submit).unwrap();
    assert_eq!(publication.current_committed(), None);
    publication
        .advance(second, Event::TransportComplete)
        .unwrap();
    assert_eq!(publication.last_committed(), Some(FIRST));
    publication.advance(second, Event::Commit).unwrap();
    assert_eq!(publication.state(second), Ok(State::HostCommitted));
    assert_eq!(publication.last_committed(), Some(second));
    assert_eq!(publication.current_committed(), Some(second));
}

#[test]
fn submitted_candidate_blocks_admission_until_a_terminal_transition() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    let second = FIRST.checked_next().unwrap();

    for (event, state) in [
        (Event::Submit, State::Submitted),
        (Event::TransportComplete, State::TransportCompleted),
    ] {
        publication.advance(FIRST, event).unwrap();
        assert_eq!(
            prepare(&mut publication, second),
            Err(PublicationError::UnsettledCandidate)
        );
        assert_eq!(publication.state(FIRST), Ok(state));
        assert_eq!(
            publication.state(second),
            Err(PublicationError::IdentityMismatch)
        );
        assert_eq!(publication.last_committed(), None);
    }
}

#[test]
fn settled_failure_allows_the_previously_rejected_sequence() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    publication.advance(FIRST, Event::Submit).unwrap();
    let second = FIRST.checked_next().unwrap();
    assert_eq!(
        prepare(&mut publication, second),
        Err(PublicationError::UnsettledCandidate)
    );

    let failure = PublicationFailure::BeforeVisibleMutation;
    publication.advance(FIRST, Event::Fail(failure)).unwrap();
    assert_eq!(publication.state(FIRST), Ok(State::Failed(failure)));
    admit(&mut publication, second);
    assert_eq!(publication.state(second), Ok(State::Prepared));
    assert_eq!(
        publication.advance(FIRST, Event::TransportComplete),
        Err(PublicationError::Stale)
    );
}

#[test]
fn possibly_visible_failure_retains_history_and_uncertainty_until_commit() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    commit_prepared(&mut publication, FIRST);

    let second = FIRST.checked_next().unwrap();
    admit(&mut publication, second);
    publication.advance(second, Event::Submit).unwrap();
    let failure = PublicationFailure::PossiblyVisible;
    publication.advance(second, Event::Fail(failure)).unwrap();
    assert_eq!(publication.state(second), Ok(State::Failed(failure)));
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), None);
    assert!(publication.visibility_uncertain());

    let third = second.checked_next().unwrap();
    admit(&mut publication, third);
    assert!(publication.visibility_uncertain());
    publication
        .advance(
            third,
            Event::Fail(PublicationFailure::BeforeVisibleMutation),
        )
        .unwrap();
    assert_eq!(publication.current_committed(), None);
    assert!(publication.visibility_uncertain());

    let generation = publication.invalidate().unwrap();
    let replacement = FrameIdentity {
        generation,
        ..FIRST
    };
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert!(publication.visibility_uncertain());
    admit(&mut publication, replacement);
    commit_prepared(&mut publication, replacement);
    assert_eq!(publication.current_committed(), Some(replacement));
    assert!(!publication.visibility_uncertain());
}

#[test]
fn invalidated_submission_stays_unsettled_and_its_completion_is_stale() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    commit_prepared(&mut publication, FIRST);
    let pending = FIRST.checked_next().unwrap();
    admit(&mut publication, pending);
    publication.advance(pending, Event::Submit).unwrap();

    let generation = publication.invalidate().unwrap();
    let replacement = FrameIdentity {
        generation,
        ..FIRST
    };
    assert_eq!(publication.state(pending), Ok(State::Stale));
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), None);
    assert_eq!(
        publication.state(replacement),
        Err(PublicationError::IdentityMismatch)
    );
    let settled = Event::Fail(PublicationFailure::BeforeVisibleMutation);
    assert_eq!(
        publication.advance(FIRST, settled),
        Err(PublicationError::Stale)
    );
    for event in [Event::TransportComplete, Event::Commit] {
        assert_eq!(
            publication.advance(pending, event),
            Err(PublicationError::Stale)
        );
    }
    assert_eq!(
        prepare(&mut publication, replacement),
        Err(PublicationError::UnsettledCandidate)
    );
    assert!(publication.visibility_uncertain());

    publication.advance(pending, settled).unwrap();
    admit(&mut publication, replacement);
    assert_eq!(
        publication.advance(pending, settled),
        Err(PublicationError::Stale)
    );
    assert_eq!(publication.state(replacement), Ok(State::Prepared));
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), None);
    assert!(!publication.visibility_uncertain());
}

#[test]
fn invalid_and_duplicate_transitions_preserve_candidate_and_commit() {
    let mut publication = Publication::new(FIRST.generation);
    admit(&mut publication, FIRST);
    commit_prepared(&mut publication, FIRST);

    let second = FIRST.checked_next().unwrap();
    admit(&mut publication, second);
    for event in [
        Event::TransportComplete,
        Event::Commit,
        Event::Fail(PublicationFailure::PossiblyVisible),
    ] {
        assert_eq!(
            publication.advance(second, event),
            Err(PublicationError::InvalidTransition)
        );
        assert_eq!(publication.state(second), Ok(State::Prepared));
        assert_eq!(publication.last_committed(), Some(FIRST));
        assert_eq!(publication.current_committed(), Some(FIRST));
        assert!(!publication.visibility_uncertain());
    }
    assert_eq!(
        prepare(&mut publication, second),
        Err(PublicationError::Stale)
    );
    assert_eq!(publication.state(second), Ok(State::Prepared));

    let failure = PublicationFailure::BeforeVisibleMutation;
    publication.advance(second, Event::Fail(failure)).unwrap();
    assert_eq!(
        publication.advance(second, Event::Fail(PublicationFailure::PossiblyVisible)),
        Err(PublicationError::InvalidTransition)
    );
    assert_eq!(publication.state(second), Ok(State::Failed(failure)));
    assert_eq!(publication.last_committed(), Some(FIRST));
    assert_eq!(publication.current_committed(), Some(FIRST));
    assert!(!publication.visibility_uncertain());
}

#[test]
fn external_producer_can_supply_an_owned_backend_specific_payload() {
    struct SyntheticProducer(PresentationCapabilities);
    #[derive(Debug, PartialEq, Eq)]
    struct SyntheticPayload(String);

    impl FrameProducer for SyntheticProducer {
        type Payload = SyntheticPayload;
        type Error = ValidationError;

        fn capabilities(&self) -> &PresentationCapabilities {
            &self.0
        }

        fn prepare_frame(
            &mut self,
            request: &FrameRequest,
        ) -> Result<PreparedFrame<Self::Payload>, Self::Error> {
            let extent = request.extent();
            let hit_region = HitRegion::new(extent, [span(0, 0, 1)])?;
            let metadata =
                FrameMetadata::new(extent, Some(Rect::new(0, 0, 1, 1)), None, hit_region)?;
            Ok(PreparedFrame {
                identity: request.identity(),
                metadata,
                payload: SyntheticPayload(String::from("synthetic")),
            })
        }
    }

    let request = request(FIRST);
    let frame = {
        let mut producer =
            SyntheticProducer(PresentationCapabilities::new(extent(), vec![IDLE], None).unwrap());
        assert!(producer.capabilities().supports(request.intent()));
        assert_eq!(producer.capabilities().frame_period(), None);
        producer.prepare_frame(&request).unwrap()
    };
    let mut publication = Publication::new(FIRST.generation);
    publication.prepare(&request, &frame).unwrap();
    assert_eq!(publication.state(FIRST), Ok(State::Prepared));
    assert!(frame.metadata.hit_region().contains(Point::new(0, 0)));
    assert_eq!(frame.payload, SyntheticPayload(String::from("synthetic")));
}
