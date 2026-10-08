//! Absolute layer requests. No pointer ownership, X11 sequence gates, or retries.
use std::time::{Duration, Instant};

pub const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Above,
    Normal,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedLayer {
    Above,
    Normal,
    Below,
    Conflict,
}
impl ObservedLayer {
    pub fn from_flags(above: bool, below: bool) -> Self {
        match (above, below) {
            (false, false) => Self::Normal,
            (true, false) => Self::Above,
            (false, true) => Self::Below,
            (true, true) => Self::Conflict,
        }
    }
    pub fn above(self) -> bool {
        matches!(self, Self::Above | Self::Conflict)
    }
    pub fn below(self) -> bool {
        matches!(self, Self::Below | Self::Conflict)
    }
    pub fn matches(self, target: Layer) -> bool {
        matches!(
            (self, target),
            (Self::Above, Layer::Above)
                | (Self::Normal, Layer::Normal)
                | (Self::Below, Layer::Below)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub above: bool,
    pub below: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutation {
    Remove(Flags),
    AddAbove,
    AddBelow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    AwaitRemoval,
    AwaitFinal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingLayer {
    pub phase: Phase,
    pub deadline: Instant,
    pub required: Flags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Busy,
    AlreadyMatches,
    Send(Mutation),
    Waiting,
    Confirmed,
    TimedOut(Phase),
}

#[derive(Debug, Default)]
pub struct LayerController {
    pub desired: Option<Layer>,
    /// Last readable snapshot, not a substitute for a fresh confirmation read.
    pub observed: Option<ObservedLayer>,
    pending: Option<PendingLayer>,
}
impl LayerController {
    pub fn pending(&self) -> Option<PendingLayer> {
        self.pending
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|pending| pending.deadline)
    }
    pub fn abort(&mut self) -> Option<PendingLayer> {
        self.pending.take()
    }
    pub fn begin(&mut self, target: Layer, observed: ObservedLayer, accepted: Instant) -> Step {
        if self.pending.is_some() {
            return Step::Busy;
        }
        self.desired = Some(target);
        self.observed = Some(observed);
        if observed.matches(target) {
            return Step::AlreadyMatches;
        }
        let required = Flags {
            above: observed.above() || target == Layer::Above,
            below: observed.below() || target == Layer::Below,
        };
        let (phase, mutation) = match target {
            Layer::Above if observed.below() => (
                Phase::AwaitRemoval,
                Mutation::Remove(Flags {
                    above: false,
                    below: true,
                }),
            ),
            Layer::Below if observed.above() => (
                Phase::AwaitRemoval,
                Mutation::Remove(Flags {
                    above: true,
                    below: false,
                }),
            ),
            Layer::Above => (Phase::AwaitFinal, Mutation::AddAbove),
            Layer::Below => (Phase::AwaitFinal, Mutation::AddBelow),
            Layer::Normal => (Phase::AwaitFinal, Mutation::Remove(required)),
        };
        self.pending = Some(PendingLayer {
            phase,
            deadline: accepted + CONFIRMATION_TIMEOUT,
            required,
        });
        Step::Send(mutation)
    }
    /// Only a fresh readable property enters this method. Event metadata never does.
    pub fn observe(&mut self, observed: ObservedLayer, now: Instant) -> Step {
        self.observed = Some(observed);
        let Some(pending) = self.pending else {
            return Step::Waiting;
        };
        let target = self.desired.expect("pending layer has a target");
        if observed.matches(target) {
            self.pending = None;
            return Step::Confirmed;
        }
        if now >= pending.deadline {
            self.pending = None;
            return Step::TimedOut(pending.phase);
        }
        if pending.phase == Phase::AwaitRemoval {
            let addition = match target {
                Layer::Above if !observed.below() => Some(Mutation::AddAbove),
                Layer::Below if !observed.above() => Some(Mutation::AddBelow),
                _ => None,
            };
            if let Some(addition) = addition {
                self.pending.as_mut().unwrap().phase = Phase::AwaitFinal;
                return Step::Send(addition);
            }
        }
        Step::Waiting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_targets_and_observations_produce_absolute_bounded_transitions() {
        let now = Instant::now();
        for target in [Layer::Above, Layer::Normal, Layer::Below] {
            for observed in [
                ObservedLayer::Normal,
                ObservedLayer::Above,
                ObservedLayer::Below,
                ObservedLayer::Conflict,
            ] {
                let mut controller = LayerController::default();
                let first = controller.begin(target, observed, now);
                if observed.matches(target) {
                    assert_eq!(first, Step::AlreadyMatches);
                    assert!(controller.deadline().is_none());
                    continue;
                }
                let expected = match (target, observed) {
                    (Layer::Above, ObservedLayer::Below | ObservedLayer::Conflict) => {
                        Mutation::Remove(Flags {
                            above: false,
                            below: true,
                        })
                    }
                    (Layer::Below, ObservedLayer::Above | ObservedLayer::Conflict) => {
                        Mutation::Remove(Flags {
                            above: true,
                            below: false,
                        })
                    }
                    (Layer::Above, _) => Mutation::AddAbove,
                    (Layer::Below, _) => Mutation::AddBelow,
                    (Layer::Normal, _) => Mutation::Remove(Flags {
                        above: observed.above(),
                        below: observed.below(),
                    }),
                };
                assert_eq!(first, Step::Send(expected));
                assert_eq!(controller.observe(observed, now), Step::Waiting);
                if controller.pending().unwrap().phase == Phase::AwaitRemoval {
                    let add = if target == Layer::Above {
                        Mutation::AddAbove
                    } else {
                        Mutation::AddBelow
                    };
                    assert_eq!(
                        controller.observe(ObservedLayer::Normal, now),
                        Step::Send(add)
                    );
                    for changed in [ObservedLayer::Conflict, observed, ObservedLayer::Normal] {
                        assert_eq!(controller.observe(changed, now), Step::Waiting);
                    }
                }
                let final_state = match target {
                    Layer::Above => ObservedLayer::Above,
                    Layer::Below => ObservedLayer::Below,
                    Layer::Normal => ObservedLayer::Normal,
                };
                assert_eq!(controller.observe(final_state, now), Step::Confirmed);
                assert!(controller.deadline().is_none());
            }
        }
    }

    #[test]
    fn conflict_removal_can_finish_without_redundant_addition() {
        let now = Instant::now();
        let mut controller = LayerController::default();
        controller.begin(Layer::Above, ObservedLayer::Conflict, now);
        assert_eq!(
            controller.observe(ObservedLayer::Above, now),
            Step::Confirmed
        );
    }

    #[test]
    fn busy_preserves_target_snapshot_phase_and_deadline() {
        let now = Instant::now();
        let mut controller = LayerController::default();
        controller.begin(Layer::Above, ObservedLayer::Below, now);
        let pending = controller.pending();
        assert_eq!(
            controller.begin(Layer::Normal, ObservedLayer::Normal, now),
            Step::Busy
        );
        assert_eq!(controller.desired, Some(Layer::Above));
        assert_eq!(controller.observed, Some(ObservedLayer::Below));
        assert_eq!(controller.pending(), pending);
    }

    #[test]
    fn deadline_is_total_final_read_can_succeed_and_late_changes_never_resurrect() {
        let now = Instant::now();
        let deadline = now + CONFIRMATION_TIMEOUT;
        let mut controller = LayerController::default();
        controller.begin(Layer::Above, ObservedLayer::Below, now);
        for millis in 0..900 {
            assert_eq!(
                controller.observe(ObservedLayer::Below, now + Duration::from_millis(millis)),
                Step::Waiting
            );
            assert_eq!(controller.deadline(), Some(deadline));
        }
        assert_eq!(
            controller.observe(ObservedLayer::Normal, deadline - Duration::from_millis(1)),
            Step::Send(Mutation::AddAbove)
        );
        assert_eq!(controller.deadline(), Some(deadline));
        assert_eq!(
            controller.observe(ObservedLayer::Normal, deadline),
            Step::TimedOut(Phase::AwaitFinal)
        );
        assert_eq!(
            controller.observe(ObservedLayer::Above, deadline),
            Step::Waiting
        );
        assert!(controller.deadline().is_none());
        controller.begin(Layer::Below, ObservedLayer::Above, deadline);
        assert_eq!(
            controller.observe(ObservedLayer::Below, deadline + CONFIRMATION_TIMEOUT),
            Step::Confirmed
        );
    }

    #[test]
    fn removal_at_expiry_cannot_start_addition_and_abort_clears_timer() {
        let now = Instant::now();
        let mut controller = LayerController::default();
        controller.begin(Layer::Above, ObservedLayer::Below, now);
        assert_eq!(
            controller.observe(ObservedLayer::Normal, now + CONFIRMATION_TIMEOUT),
            Step::TimedOut(Phase::AwaitRemoval)
        );
        controller.begin(Layer::Below, ObservedLayer::Normal, now);
        assert!(controller.abort().is_some());
        assert!(controller.abort().is_none());
        assert!(controller.deadline().is_none());
        assert_eq!(controller.observe(ObservedLayer::Below, now), Step::Waiting);
    }
}
