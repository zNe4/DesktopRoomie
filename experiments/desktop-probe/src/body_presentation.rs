//! Owner-side frame requests, preparation and successful-publication timing.

use std::time::{Duration, Instant};

use crate::presentation::procedural::{Palette, ProceduralProducer};
use crate::presentation::synthetic::SyntheticProducer;
use crate::presentation::{
    ActionIntent, ClientExtent, FrameProducer, FrameRequest, PreparedFrame,
    PresentationCapabilities,
};
use crate::x11::render::FrameGeometry;
use crate::HostError;

pub(crate) struct BodyPresentation<P> {
    producer: P,
    intent: ActionIntent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameKind {
    Initial,
    Redraw,
    Scheduled,
}

impl<P> BodyPresentation<P>
where
    P: FrameProducer<Payload = Vec<u8>>,
    P::Error: Into<HostError>,
{
    pub(crate) fn new(producer: P) -> Result<Self, HostError> {
        let intent = *producer
            .capabilities()
            .supported_intents()
            .first()
            .ok_or("Body producer supports no presentation intents")?;
        Ok(Self { producer, intent })
    }

    pub(crate) fn initial_extent(&self) -> ClientExtent {
        self.producer.capabilities().initial_extent()
    }

    pub(crate) fn timing(&self) -> FrameTiming {
        FrameTiming::new(self.producer.capabilities().frame_period())
    }

    /// Construct the authoritative request without reserving a publication
    /// sequence. An unpublished sequence may be requested again later.
    pub(crate) fn frame_request(
        &self,
        kind: FrameKind,
        geometry: &FrameGeometry,
        timing: &mut FrameTiming,
        now: impl FnOnce() -> Instant,
    ) -> Result<FrameRequest, HostError> {
        let result = (|| {
            if timing.stopped {
                return Err("Body presentation has stopped".into());
            }
            if !self.producer.capabilities().supports(self.intent) {
                return Err("Body producer no longer supports the selected intent".into());
            }
            if (kind == FrameKind::Initial) != geometry.committed().is_none() {
                return Err("Frame request does not match the body publication phase".into());
            }
            let (elapsed, delta) = timing.request_times(kind, now())?;
            geometry.frame_request(self.intent, elapsed, delta)
        })();
        if result.is_err() {
            timing.stop();
        }
        result
    }

    pub(crate) fn prepare_with(
        &mut self,
        kind: FrameKind,
        geometry: &FrameGeometry,
        timing: &mut FrameTiming,
        now: impl FnOnce() -> Instant,
    ) -> Result<(FrameRequest, PreparedFrame<Vec<u8>>), HostError> {
        let request = self.frame_request(kind, geometry, timing, now)?;
        match self.producer.prepare_frame(&request).map_err(Into::into) {
            Ok(frame) => Ok((request, frame)),
            Err(error) => {
                timing.stop();
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn present_with(
        &mut self,
        kind: FrameKind,
        geometry: &mut FrameGeometry,
        timing: &mut FrameTiming,
        eligible_after: bool,
        mut now: impl FnMut() -> Instant,
        publish: impl FnOnce(
            &mut FrameGeometry,
            &FrameRequest,
            PreparedFrame<Vec<u8>>,
        ) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        let (request, frame) = self.prepare_with(kind, geometry, timing, &mut now)?;
        timing.publish_with(&request, eligible_after, now, || {
            publish(geometry, &request, frame)?;
            Ok(true)
        })
    }
}

#[cfg(test)]
impl BodyPresentation<ProceduralProducer> {
    pub(crate) fn toggle_palette(&mut self) -> Palette {
        self.producer.toggle_palette()
    }
}

pub(crate) enum BodyProducer {
    Procedural(ProceduralProducer),
    Synthetic(SyntheticProducer),
}

impl BodyPresentation<BodyProducer> {
    pub(crate) fn click_palette(&mut self) -> Option<Palette> {
        match &mut self.producer {
            BodyProducer::Procedural(producer) => Some(producer.toggle_palette()),
            BodyProducer::Synthetic(_) => None,
        }
    }
}

impl FrameProducer for BodyProducer {
    type Payload = Vec<u8>;
    type Error = HostError;

    fn capabilities(&self) -> &PresentationCapabilities {
        match self {
            Self::Procedural(producer) => producer.capabilities(),
            Self::Synthetic(producer) => producer.capabilities(),
        }
    }

    fn prepare_frame(
        &mut self,
        request: &FrameRequest,
    ) -> Result<PreparedFrame<Vec<u8>>, HostError> {
        match self {
            Self::Procedural(producer) => Ok(producer.prepare_frame(request)?),
            Self::Synthetic(producer) => producer.prepare_frame(request),
        }
    }
}

/// Elapsed active time follows eligibility for every producer. A frame period
/// also lets an eligible session arm one recurring deadline.
pub(crate) struct FrameTiming {
    period: Option<Duration>,
    last_observed: Option<Instant>,
    active_since: Option<Instant>,
    deadline: Option<Instant>,
    elapsed: Duration,
    last_request_elapsed: Duration,
    stopped: bool,
}

impl FrameTiming {
    /// Only a successful checked publication advances presentation timing.
    /// The completion clock observation stays after the native commitment.
    /// A false publication result means ordinary pre-submission invalidation:
    /// leave timing untouched and do not read the completion clock.
    pub(crate) fn publish_with(
        &mut self,
        request: &FrameRequest,
        eligible_after: bool,
        mut now: impl FnMut() -> Instant,
        publish: impl FnOnce() -> Result<bool, HostError>,
    ) -> Result<(), HostError> {
        let result = (|| {
            if self.stopped {
                return Err("Body presentation has stopped".into());
            }
            if !publish()? {
                return Ok(());
            }
            self.presented(request.active_elapsed(), now(), eligible_after)
        })();
        if result.is_err() {
            self.stop();
        }
        result
    }

    pub(crate) fn new(period: Option<Duration>) -> Self {
        Self {
            period,
            last_observed: None,
            active_since: None,
            deadline: None,
            elapsed: Duration::ZERO,
            last_request_elapsed: Duration::ZERO,
            stopped: false,
        }
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(crate) fn due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    #[cfg(feature = "managed-supervisor-fixture")]
    pub(crate) fn consume_opportunity(&mut self, now: Instant) -> bool {
        if self.due(now) {
            self.deadline = None;
            true
        } else {
            false
        }
    }

    /// Backpressure suppresses request deadlines while active time continues.
    #[cfg(feature = "managed-supervisor-fixture")]
    pub(crate) fn update_opportunity(&mut self, now: Instant, idle: bool) -> Result<(), HostError> {
        if !idle || self.active_since.is_none() {
            self.deadline = None;
        } else if self.deadline.is_none() {
            self.observe(now)?;
            self.deadline = self
                .period
                .map(|period| {
                    now.checked_add(period)
                        .ok_or("Frame period exceeds supported monotonic deadline")
                })
                .transpose()?;
        }
        Ok(())
    }

    pub(crate) fn stop(&mut self) {
        self.stopped = true;
        self.active_since = None;
        self.deadline = None;
    }

    pub(crate) fn update_eligibility(
        &mut self,
        now: Instant,
        eligible: bool,
    ) -> Result<(), HostError> {
        let result = self.update(now, eligible);
        if result.is_err() {
            self.stop();
        }
        result
    }

    fn update(&mut self, now: Instant, eligible: bool) -> Result<(), HostError> {
        if self.stopped {
            return Ok(());
        }
        self.observe(now)?;
        match (self.active_since, eligible) {
            (None, true) => {
                if let Some(period) = self.period {
                    self.deadline = Some(
                        now.checked_add(period)
                            .ok_or("Frame period exceeds supported monotonic deadline")?,
                    );
                }
                self.active_since = Some(now);
            }
            (Some(_), false) => {
                self.elapsed = self.elapsed_at(now)?;
                self.active_since = None;
                self.deadline = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn observe(&mut self, now: Instant) -> Result<(), HostError> {
        if self.last_observed.is_some_and(|last| now < last) {
            return Err("Frame clock moved backwards".into());
        }
        self.last_observed = Some(now);
        Ok(())
    }

    fn elapsed_at(&self, now: Instant) -> Result<Duration, HostError> {
        let Some(since) = self.active_since else {
            return Ok(self.elapsed);
        };
        let interval = now
            .checked_duration_since(since)
            .ok_or("Frame clock moved backwards during active presentation")?;
        self.elapsed
            .checked_add(interval)
            .ok_or_else(|| "Frame active elapsed time exhausted".into())
    }

    fn request_times(
        &mut self,
        kind: FrameKind,
        now: Instant,
    ) -> Result<(Duration, Duration), HostError> {
        self.observe(now)?;
        if kind == FrameKind::Initial {
            return Ok((Duration::ZERO, Duration::ZERO));
        }
        let elapsed = self.elapsed_at(now)?;
        let delta = if kind == FrameKind::Scheduled {
            if !self.due(now) {
                return Err("No recurring frame is due".into());
            }
            elapsed
                .checked_sub(self.last_request_elapsed)
                .ok_or("Frame active elapsed time moved backwards")?
                .min(self.period.expect("recurring frame period"))
        } else {
            Duration::ZERO
        };
        Ok((elapsed, delta))
    }

    fn presented(
        &mut self,
        request_elapsed: Duration,
        now: Instant,
        eligible_after: bool,
    ) -> Result<(), HostError> {
        self.observe(now)?;
        self.elapsed = self.elapsed_at(now)?;
        self.last_request_elapsed = request_elapsed;
        // Rearm from successful presentation, even after an overrun or a
        // click. A resumed session also starts a full new period.
        self.active_since = None;
        self.deadline = None;
        self.update(now, eligible_after)
    }
}

#[cfg(test)]
#[path = "probe_tests/body_presentation_tests.rs"]
mod tests;
