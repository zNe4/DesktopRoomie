//! Bounded observations of the existing owner. No capture result feeds D1,
//! FrameTiming, the worker, or the checked publication transaction.
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::time::Duration;

use x11rb::protocol::shape::GetRectanglesReply;
use x11rb::protocol::xproto::{ClipOrdering, GetImageReply, ImageOrder, MapState, Rectangle};

use crate::presentation::gpu_handoff::{JobIdentity, ReadyFrame};
use crate::presentation::synthetic::SyntheticProducer;
use crate::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameMetadata, FrameProducer,
    FrameRequest, Point, Rect, Size,
};

const SIDE: usize = 160;
const IMAGE: usize = SIDE * SIDE * 4;
const MASK: usize = SIDE * SIDE / 8;
const RECTANGLES: usize = SIDE * SIDE;
const SAMPLES: usize = 64;
const EVENTS: usize = 512;
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;
const SAMPLE_BYTES: usize = 2 * IMAGE + MASK + RECTANGLES * 8;

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    Startup = 1,
    Expose,
    Admit,
    Frame,
    InvalidFrame,
    Obsolete,
    Withdrawn,
    Rejected,
    Failed,
    Unresolved,
    Ready,
    CanceledPending,
    CanceledReady,
    Expired,
    Stale,
    Selected,
    Refused,
    Admitted,
    Skipped,
    PublicationFailed,
    Committed,
    Unusable,
    Observed,
    Shutdown,
    RefusedExpired,
    D1Obsolete,
}

const KINDS: [Kind; 26] = [
    Kind::Startup,
    Kind::Expose,
    Kind::Admit,
    Kind::Frame,
    Kind::InvalidFrame,
    Kind::Obsolete,
    Kind::Withdrawn,
    Kind::Rejected,
    Kind::Failed,
    Kind::Unresolved,
    Kind::Ready,
    Kind::CanceledPending,
    Kind::CanceledReady,
    Kind::Expired,
    Kind::Stale,
    Kind::Selected,
    Kind::Refused,
    Kind::Admitted,
    Kind::Skipped,
    Kind::PublicationFailed,
    Kind::Committed,
    Kind::Unusable,
    Kind::Observed,
    Kind::Shutdown,
    Kind::RefusedExpired,
    Kind::D1Obsolete,
];

#[derive(Clone, Copy)]
struct Event {
    kind: Kind,
    at: Option<Duration>,
    job: Option<JobIdentity>,
    admission: Option<(FrameRequest, Duration)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub identity: FrameIdentity,
    pub body: u32,
    pub pixmap: u32,
    pub origin: Point,
    pub enabled_after: u64,
    pub input_blocked: bool,
    pub temporary_empty_confirmed: bool,
    pub interaction_suppressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveBody {
    pub map_state: u8,
    pub visual: u32,
    pub depth: u8,
    pub override_redirect: bool,
    pub root: u32,
    pub width: u16,
    pub height: u16,
    pub border: u16,
    pub same_screen: bool,
    pub origin: Point,
}

impl LiveBody {
    pub(crate) fn validate(self, target: Target, visual: u32) -> Result<()> {
        if self.map_state != u8::from(MapState::VIEWABLE)
            || self.visual != visual
            || self.depth != 32
            || self.override_redirect
            || self.root == 0
            || (self.width, self.height, self.border) != (160, 160, 0)
            || !self.same_screen
            || self.origin != target.origin
        {
            return Err("live body visibility, geometry or placement changed");
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub visual: u32,
    pub pict_format: u32,
    pub masks: [u16; 4],
    pub shifts: [u16; 4],
    pub format_depth: u8,
    pub bpp: u8,
    pub image_order: u8,
    pub pad: u8,
    pub reply_depth: u8,
    pub reply_visual: u32,
    pub reply_bytes: u32,
}

impl Layout {
    fn validate(self, len: usize) -> Result<()> {
        if self.visual == 0
            || self.pict_format == 0
            || self.masks != [255; 4]
            || self.shifts != [0, 8, 16, 24]
            || self.format_depth != 32
            || self.reply_depth != 32
            || self.reply_visual != 0
            || self.bpp != 32
            || self.pad != 32
            || self.image_order != u8::from(ImageOrder::LSB_FIRST)
        {
            return Err("unsupported backing-pixmap layout");
        }
        let pad = usize::from(self.pad);
        let stride = SIDE
            .checked_mul(self.bpp.into())
            .and_then(|bits| bits.checked_next_multiple_of(pad))
            .ok_or("image stride overflow")?
            / 8;
        if stride != 640
            || stride.checked_mul(SIDE) != Some(len)
            || len != IMAGE
            || usize::try_from(self.reply_bytes).ok() != Some(len)
        {
            return Err("incorrect backing-pixmap reply length");
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Readback {
    pub target: Target,
    pub layout: Layout,
    pub live: LiveBody,
    pub image_sequence: u64,
    pub image_reply: u16,
    pub shape_sequence: u64,
    pub shape_reply: u16,
    pub image_at: Duration,
    pub shape_at: Duration,
    pub ordering: u8,
    pub shape_words: u32,
}

pub(crate) struct NativeReplies {
    pub layout: Layout,
    pub image_sequence: u64,
    pub image_at: Duration,
    pub image: GetImageReply,
    pub shape_sequence: u64,
    pub shape_at: Duration,
    pub shape: GetRectanglesReply,
}

impl Readback {
    fn validate(self, rectangle_count: usize) -> Result<()> {
        self.live.validate(self.target, self.layout.visual)?;
        if self.target.body == 0
            || self.target.pixmap == 0
            || self.target.body == self.target.pixmap
            || self.target.input_blocked
            || self.target.temporary_empty_confirmed
            || self.target.enabled_after == 0
            || self.target.enabled_after >= self.image_sequence
            || self.image_sequence >= self.shape_sequence
            || self.image_sequence as u16 != self.image_reply
            || self.shape_sequence as u16 != self.shape_reply
            || self.image_at > self.shape_at
            || u32::try_from(rectangle_count)
                .ok()
                .and_then(|count| count.checked_mul(2))
                != Some(self.shape_words)
            || ![
                ClipOrdering::UNSORTED,
                ClipOrdering::Y_SORTED,
                ClipOrdering::YX_SORTED,
                ClipOrdering::YX_BANDED,
            ]
            .map(u8::from)
            .contains(&self.ordering)
        {
            return Err("invalid native observation sequence or target");
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Sample {
    job: JobIdentity,
    bounds: Option<Rect>,
    anchor: Option<Point>,
    readback: Option<Readback>,
    rectangles: usize,
}

pub(crate) struct Capture {
    events: Vec<Event>,
    samples: Vec<Sample>,
    data: Vec<u8>,
    pending: bool,
    failure: Option<&'static str>,
}

fn allocated<T>(count: usize, budget: &mut usize) -> Result<Vec<T>> {
    let required = size_of::<T>()
        .checked_mul(count)
        .ok_or("capture size overflow")?;
    if required > *budget {
        return Err("capture allocation exceeds budget");
    }
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(count)
        .map_err(|_| "capture allocation failed")?;
    let actual = size_of::<T>()
        .checked_mul(buffer.capacity())
        .ok_or("capture capacity overflow")?;
    *budget = budget
        .checked_sub(actual)
        .ok_or("capture capacity exceeds budget")?;
    Ok(buffer)
}

fn append<T>(records: &mut Vec<T>, limit: usize, record: T) -> Result<()> {
    if records.len() >= limit || records.len() == records.capacity() {
        return Err("capture record limit reached");
    }
    records.push(record);
    Ok(())
}

impl Capture {
    pub fn new() -> Self {
        Self::allocate(MAX_BYTES).unwrap_or_else(|error| Self {
            events: Vec::new(),
            samples: Vec::new(),
            data: Vec::new(),
            pending: false,
            failure: Some(error),
        })
    }

    fn allocate(budget: usize) -> Result<Self> {
        let mut budget = budget
            .checked_sub(size_of::<Self>())
            .ok_or("capture budget too small")?;
        Ok(Self {
            events: allocated(EVENTS, &mut budget)?,
            samples: allocated(SAMPLES, &mut budget)?,
            data: allocated(
                SAMPLES
                    .checked_mul(SAMPLE_BYTES)
                    .ok_or("sample capacity overflow")?,
                &mut budget,
            )?,
            pending: false,
            failure: None,
        })
    }

    pub fn fail(&mut self, error: &'static str) {
        self.failure.get_or_insert(error);
    }

    pub fn available(&self) -> bool {
        self.failure.is_none()
    }

    pub fn event(&mut self, kind: Kind, job: Option<JobIdentity>, at: Option<Duration>) {
        self.record(Event {
            kind,
            job,
            at,
            admission: None,
        });
    }

    pub fn admitted(
        &mut self,
        job: JobIdentity,
        at: Option<Duration>,
        request: FrameRequest,
        deadline: Option<Duration>,
    ) {
        match (at, deadline) {
            (Some(at), Some(deadline)) => self.record(Event {
                kind: Kind::Admit,
                at: Some(at),
                job: Some(job),
                admission: Some((request, deadline)),
            }),
            _ => self.fail("missing original owner clock offset"),
        }
    }

    fn record(&mut self, event: Event) {
        if self.available() {
            if let Err(error) = append(&mut self.events, EVENTS, event) {
                self.fail(error);
            }
        }
    }

    pub fn select(&mut self, ready: &ReadyFrame<Vec<u8>>, deadline: Option<Duration>) {
        if !self.available() {
            return;
        }
        if let Err(error) = self.select_inner(ready, deadline) {
            self.fail(error);
        }
    }

    fn select_inner(
        &mut self,
        ready: &ReadyFrame<Vec<u8>>,
        deadline: Option<Duration>,
    ) -> Result<()> {
        let original = self
            .events
            .iter()
            .rev()
            .find(|event| event.kind == Kind::Admit && event.job == Some(ready.identity))
            .and_then(|event| event.admission);
        if original != deadline.map(|deadline| (ready.request, deadline)) || original.is_none() {
            return Err("selected candidate does not match its original request and deadline");
        }
        let frame = &ready.frame;
        if self.pending || self.samples.len() == SAMPLES {
            return Err("pending sample or commitment sample limit reached");
        }
        if frame.identity != ready.identity.frame || frame.payload.len() != IMAGE {
            return Err("invalid selected frame identity or length");
        }
        let metadata = &frame.metadata;
        let mask = metadata_mask(metadata)?;
        let end = self
            .data
            .len()
            .checked_add(SAMPLE_BYTES)
            .ok_or("sample offset overflow")?;
        if end > self.data.capacity() {
            return Err("capture payload capacity exhausted");
        }
        self.data.extend_from_slice(&frame.payload);
        self.data.extend_from_slice(&mask);
        self.data.resize(end, 0);
        append(
            &mut self.samples,
            SAMPLES,
            Sample {
                job: ready.identity,
                bounds: metadata.visible_bounds(),
                anchor: metadata.anchor(),
                readback: None,
                rectangles: 0,
            },
        )?;
        self.pending = true;
        self.event(Kind::Selected, Some(ready.identity), None);
        Ok(())
    }

    pub fn pending_job(&self) -> Option<JobIdentity> {
        self.available().then_some(())?;
        self.pending
            .then(|| self.samples.last().expect("selected sample").job)
    }

    pub fn discard(&mut self) {
        if self.pending && self.available() {
            self.samples.pop();
            self.data.truncate(self.data.len() - SAMPLE_BYTES);
            self.pending = false;
        }
    }

    /// The owner calls this only after checked publication and timing/input
    /// restoration. A polled event remains queued for ordinary owner handling.
    pub fn observe_pending<T>(
        &mut self,
        buffered: &mut Option<T>,
        mut peek: impl FnMut() -> std::result::Result<Option<T>, crate::HostError>,
        target: impl Fn() -> Option<Target>,
        live: impl Fn(Target) -> std::result::Result<LiveBody, crate::HostError>,
        pair: impl FnOnce(Target) -> std::result::Result<NativeReplies, crate::HostError>,
    ) {
        let Some(job) = self.pending_job() else {
            return;
        };
        let result = (|| -> std::result::Result<(), crate::HostError> {
            let before = target()
                .filter(|t| t.identity == job.frame)
                .ok_or("selected frame lacks usable owner authority")?;
            let mut check_buffer = || -> std::result::Result<(), crate::HostError> {
                if !crate::look_ahead(buffered, &mut peek)? {
                    return Err("buffered owner event makes native attribution ambiguous".into());
                }
                Ok(())
            };
            check_buffer()?;
            let live_before = live(before)?;
            let native = pair(before)?;
            let live_after = live(before)?;
            check_buffer()?;
            let after = target().ok_or("owner target lost input readiness")?;
            self.accept_replies(before, after, live_before, live_after, native)
                .map_err(Into::into)
        })();
        if let Err(error) = result {
            eprintln!("[MANAGED EVIDENCE] {error}");
            self.fail("post-commit native observation failed or was ambiguous");
        }
    }

    fn accept_replies(
        &mut self,
        before: Target,
        after: Target,
        live_before: LiveBody,
        live_after: LiveBody,
        native: NativeReplies,
    ) -> Result<()> {
        let mut layout = native.layout;
        layout.reply_depth = native.image.depth;
        layout.reply_visual = native.image.visual;
        layout.reply_bytes = u32::try_from(native.image.data.len()).unwrap_or(u32::MAX);
        let readback = Readback {
            target: before,
            live: live_before,
            layout,
            image_sequence: native.image_sequence,
            image_reply: native.image.sequence,
            shape_sequence: native.shape_sequence,
            shape_reply: native.shape.sequence,
            image_at: native.image_at,
            shape_at: native.shape_at,
            ordering: native.shape.ordering.into(),
            shape_words: native.shape.length,
        };
        self.observe_inner(
            before,
            after,
            live_after,
            readback,
            &native.image.data,
            &native.shape.rectangles,
        )
    }

    fn observe_inner(
        &mut self,
        before: Target,
        after: Target,
        live_after: LiveBody,
        readback: Readback,
        image: &[u8],
        rectangles: &[Rectangle],
    ) -> Result<()> {
        let job = self.pending_job().ok_or("missing selected commitment")?;
        if before != after
            || before != readback.target
            || before.identity != job.frame
            || readback.live != live_after
        {
            return Err("native observation does not belong to selected commitment");
        }
        readback.validate(rectangles.len())?;
        readback.layout.validate(image.len())?;
        rectangle_mask(rectangles.iter().copied(), rectangles.len())?;
        let start = self.data.len() - SAMPLE_BYTES + IMAGE + MASK;
        self.data[start..start + IMAGE].copy_from_slice(image);
        for (rect, bytes) in rectangles
            .iter()
            .zip(self.data[start + IMAGE..].as_chunks_mut::<8>().0)
        {
            bytes[..2].copy_from_slice(&rect.x.to_le_bytes());
            bytes[2..4].copy_from_slice(&rect.y.to_le_bytes());
            bytes[4..6].copy_from_slice(&rect.width.to_le_bytes());
            bytes[6..].copy_from_slice(&rect.height.to_le_bytes());
        }
        let sample = self.samples.last_mut().expect("selected sample");
        sample.readback = Some(readback);
        sample.rectangles = rectangles.len();
        self.pending = false;
        self.event(Kind::Observed, Some(job), Some(readback.shape_at));
        Ok(())
    }
}

fn cover(mask: &mut [u8; MASK], y: usize, start: usize, end: usize) {
    for x in start..end {
        let pixel = y * SIDE + x;
        mask[pixel / 8] |= 1 << (pixel % 8);
    }
}

fn metadata_mask(metadata: &FrameMetadata) -> Result<[u8; MASK]> {
    if metadata.extent().size() != Size::new(SIDE as u32, SIDE as u32) {
        return Err("unexpected frame metadata extent");
    }
    let mut mask = [0; MASK];
    for span in metadata.hit_region().spans() {
        cover(
            &mut mask,
            span.y as usize,
            span.start_x as usize,
            span.end_x as usize,
        );
    }
    Ok(mask)
}

fn rectangle_mask(rects: impl Iterator<Item = Rectangle>, count: usize) -> Result<[u8; MASK]> {
    if count > RECTANGLES {
        return Err("native SHAPE rectangle count exceeds limit");
    }
    let mut coverage = RectangleUnion::new();
    for rectangle in rects {
        coverage.add(rectangle)?;
    }
    Ok(coverage.mask())
}

// A two-dimensional difference grid unions even highly overlapping replies
// in O(rectangles + pixels), with a maximum multiplicity of RECTANGLES.
struct RectangleUnion([[i32; SIDE + 1]; SIDE + 1]);
impl RectangleUnion {
    fn new() -> Self {
        Self([[0; SIDE + 1]; SIDE + 1])
    }

    fn add(&mut self, r: Rectangle) -> Result<()> {
        let x = usize::try_from(r.x).map_err(|_| "negative SHAPE x")?;
        let y = usize::try_from(r.y).map_err(|_| "negative SHAPE y")?;
        let right = x
            .checked_add(r.width.into())
            .ok_or("SHAPE right edge overflow")?;
        let bottom = y
            .checked_add(r.height.into())
            .ok_or("SHAPE bottom edge overflow")?;
        if right > SIDE || bottom > SIDE || x == right || y == bottom {
            return Err("invalid native SHAPE rectangle");
        }
        self.0[y][x] += 1;
        self.0[y][right] -= 1;
        self.0[bottom][x] -= 1;
        self.0[bottom][right] += 1;
        Ok(())
    }

    fn mask(self) -> [u8; MASK] {
        let mut mask = [0; MASK];
        let mut columns = [0; SIDE];
        for (y, row) in self.0[..SIDE].iter().enumerate() {
            let mut count = 0;
            for (x, (column, delta)) in columns.iter_mut().zip(row).enumerate() {
                *column += delta;
                count += *column;
                if count > 0 {
                    cover(&mut mask, y, x, x + 1);
                }
            }
        }
        mask
    }
}

impl Capture {
    pub fn cancel_pending(&mut self, identity: JobIdentity, at: Option<Duration>) {
        if !self
            .events
            .iter()
            .any(|event| event.job == Some(identity) && event.kind == Kind::CanceledPending)
        {
            self.event(Kind::CanceledPending, Some(identity), at);
        }
    }
}

// File format v1. Integers are little-endian, durations are (u64 seconds,
// u32 nanoseconds). Optional values use a single 0/1 tag. Counts are bounded
// before reading their contents. Pixels and rectangles are stored verbatim.
const HEADER: &[u8] = b"ROOMIE-MANAGED-VULKAN\x01";
const FOOTER: &[u8] = b"ROOMIE-COMPLETE\x01";

trait Encode: Write {
    fn number(&mut self, value: impl Into<u64>) -> io::Result<()> {
        self.write_all(&value.into().to_le_bytes())
    }
    fn optional<T>(
        &mut self,
        value: Option<T>,
        write: impl FnOnce(&mut Self, T) -> io::Result<()>,
    ) -> io::Result<()> {
        self.write_all(&[u8::from(value.is_some())])?;
        if let Some(value) = value {
            write(self, value)?;
        }
        Ok(())
    }
    fn time(&mut self, duration: Duration) -> io::Result<()> {
        self.number(duration.as_secs())?;
        self.write_all(&duration.subsec_nanos().to_le_bytes())
    }
    fn identity(&mut self, frame: FrameIdentity) -> io::Result<()> {
        self.number(frame.generation)?;
        self.number(frame.sequence)
    }
    fn job(&mut self, job: JobIdentity) -> io::Result<()> {
        self.number(job.session)?;
        self.number(job.epoch)?;
        self.number(job.serial)?;
        self.identity(job.frame)
    }
    fn point(&mut self, point: Point) -> io::Result<()> {
        self.write_all(&point.x.to_le_bytes())?;
        self.write_all(&point.y.to_le_bytes())
    }
    fn rect(&mut self, rect: Rect) -> io::Result<()> {
        self.point(rect.origin())?;
        self.number(rect.width)?;
        self.number(rect.height)
    }
    fn request(&mut self, request: FrameRequest, deadline: Duration) -> io::Result<()> {
        self.identity(request.identity())?;
        self.number(request.extent().size().width)?;
        self.number(request.extent().size().height)?;
        self.write_all(&[request.intent().action as u8, request.intent().facing as u8])?;
        self.time(request.active_elapsed())?;
        self.time(request.active_delta())?;
        self.time(deadline)
    }
}
impl<W: Write> Encode for W {}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

trait Decode: Read {
    fn bytes<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn number<T: TryFrom<u64>>(&mut self) -> io::Result<T> {
        T::try_from(u64::from_le_bytes(self.bytes()?)).map_err(|_| invalid("number out of range"))
    }
    fn optional<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<Option<T>> {
        match self.bytes::<1>()?[0] {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(invalid("invalid presence tag")),
        }
    }
    fn time(&mut self) -> io::Result<Duration> {
        let secs = self.number()?;
        let nanos = u32::from_le_bytes(self.bytes()?);
        if nanos >= 1_000_000_000 {
            return Err(invalid("noncanonical nanosecond field"));
        }
        Ok(Duration::new(secs, nanos))
    }
    fn identity(&mut self) -> io::Result<FrameIdentity> {
        Ok(FrameIdentity {
            generation: self.number()?,
            sequence: self.number()?,
        })
    }
    fn job(&mut self) -> io::Result<JobIdentity> {
        Ok(JobIdentity {
            session: self.number()?,
            epoch: self.number()?,
            serial: self.number()?,
            frame: self.identity()?,
        })
    }
    fn point(&mut self) -> io::Result<Point> {
        Ok(Point::new(
            i32::from_le_bytes(self.bytes()?),
            i32::from_le_bytes(self.bytes()?),
        ))
    }
    fn rect(&mut self) -> io::Result<Rect> {
        let p = self.point()?;
        Ok(Rect::new(p.x, p.y, self.number()?, self.number()?))
    }
    fn request(&mut self) -> io::Result<(FrameRequest, Duration)> {
        let frame = self.identity()?;
        let extent = ClientExtent::new(Size::new(self.number()?, self.number()?))
            .map_err(|_| invalid("invalid request extent"))?;
        let intent = match self.bytes::<2>()? {
            [0, 0] => ActionIntent {
                action: Action::Idle,
                facing: Facing::Neutral,
            },
            _ => return Err(invalid("unsupported request intent")),
        };
        let request = FrameRequest::new(frame, intent, extent, self.time()?, self.time()?)
            .map_err(|_| invalid("invalid original request timing"))?;
        Ok((request, self.time()?))
    }
}
impl<R: Read> Decode for R {}

impl Readback {
    fn write(self, w: &mut impl Encode) -> io::Result<()> {
        w.identity(self.target.identity)?;
        w.number(self.target.body)?;
        w.number(self.target.pixmap)?;
        w.point(self.target.origin)?;
        w.number(self.target.enabled_after)?;
        w.write_all(&[
            u8::from(self.target.input_blocked),
            u8::from(self.target.temporary_empty_confirmed),
            u8::from(self.target.interaction_suppressed),
        ])?;
        let l = self.layout;
        w.number(l.visual)?;
        w.number(l.pict_format)?;
        for value in l.masks.into_iter().chain(l.shifts) {
            w.number(value)?;
        }
        w.write_all(&[l.format_depth, l.bpp, l.image_order, l.pad, l.reply_depth])?;
        w.number(l.reply_visual)?;
        w.number(l.reply_bytes)?;
        w.number(self.live.visual)?;
        w.number(self.live.root)?;
        w.number(self.live.width)?;
        w.number(self.live.height)?;
        w.number(self.live.border)?;
        w.point(self.live.origin)?;
        w.write_all(&[
            self.live.map_state,
            self.live.depth,
            u8::from(self.live.override_redirect),
            u8::from(self.live.same_screen),
        ])?;
        w.number(self.image_sequence)?;
        w.number(self.image_reply)?;
        w.number(self.shape_sequence)?;
        w.number(self.shape_reply)?;
        w.time(self.image_at)?;
        w.time(self.shape_at)?;
        w.write_all(&[self.ordering])?;
        w.number(self.shape_words)
    }

    fn read(r: &mut impl Decode) -> io::Result<Self> {
        let identity = r.identity()?;
        let body = r.number()?;
        let pixmap = r.number()?;
        let origin = r.point()?;
        let enabled_after = r.number()?;
        let [input_blocked, temporary_empty_confirmed, interaction_suppressed] = r.bytes()?;
        if [
            input_blocked,
            temporary_empty_confirmed,
            interaction_suppressed,
        ]
        .into_iter()
        .any(|value| value > 1)
        {
            return Err(invalid("invalid owner input or interaction flags"));
        }
        let target = Target {
            identity,
            body,
            pixmap,
            origin,
            enabled_after,
            input_blocked: input_blocked != 0,
            temporary_empty_confirmed: temporary_empty_confirmed != 0,
            interaction_suppressed: interaction_suppressed != 0,
        };
        let visual = r.number()?;
        let pict_format = r.number()?;
        let masks = [r.number()?, r.number()?, r.number()?, r.number()?];
        let shifts = [r.number()?, r.number()?, r.number()?, r.number()?];
        let [format_depth, bpp, image_order, pad, reply_depth] = r.bytes()?;
        Ok(Self {
            target,
            layout: Layout {
                visual,
                pict_format,
                masks,
                shifts,
                format_depth,
                bpp,
                image_order,
                pad,
                reply_depth,
                reply_visual: r.number()?,
                reply_bytes: r.number()?,
            },
            live: {
                let (visual, root, width, height, border, origin) = (
                    r.number()?,
                    r.number()?,
                    r.number()?,
                    r.number()?,
                    r.number()?,
                    r.point()?,
                );
                let [map_state, depth, override_redirect, same_screen] = r.bytes()?;
                if override_redirect > 1 || same_screen > 1 {
                    return Err(invalid("invalid body observation flags"));
                }
                LiveBody {
                    map_state,
                    visual,
                    depth,
                    override_redirect: override_redirect != 0,
                    same_screen: same_screen != 0,
                    root,
                    width,
                    height,
                    border,
                    origin,
                }
            },
            image_sequence: r.number()?,
            image_reply: r.number()?,
            shape_sequence: r.number()?,
            shape_reply: r.number()?,
            image_at: r.time()?,
            shape_at: r.time()?,
            ordering: r.bytes::<1>()?[0],
            shape_words: r.number()?,
        })
    }
}

impl Event {
    fn write(self, w: &mut impl Encode) -> io::Result<()> {
        w.write_all(&[self.kind as u8])?;
        w.optional(self.job, |w, job| w.job(job))?;
        w.optional(self.at, |w, at| w.time(at))?;
        w.optional(self.admission, |w, (request, deadline)| {
            w.request(request, deadline)
        })
    }
    fn read(r: &mut impl Decode) -> io::Result<Self> {
        let tag = r.bytes::<1>()?[0];
        let kind = tag
            .checked_sub(1)
            .and_then(|tag| KINDS.get(usize::from(tag)))
            .copied()
            .ok_or_else(|| invalid("unknown diagnostic event"))?;
        Ok(Self {
            kind,
            job: r.optional(|r| r.job())?,
            at: r.optional(|r| r.time())?,
            admission: r.optional(|r| r.request())?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub bytes: u64,
    pub changed_seconds: i64,
    pub changed_nanos: u32,
}
impl FileIdentity {
    fn write(self, w: &mut impl Encode) -> io::Result<()> {
        w.number(self.device)?;
        w.number(self.inode)?;
        w.number(self.bytes)?;
        w.write_all(&self.changed_seconds.to_le_bytes())?;
        w.number(self.changed_nanos)
    }
    fn read(r: &mut impl Decode) -> io::Result<Self> {
        let id = Self {
            device: r.number()?,
            inode: r.number()?,
            bytes: r.number()?,
            changed_seconds: i64::from_le_bytes(r.bytes()?),
            changed_nanos: r.number()?,
        };
        if id.inode == 0 || id.bytes == 0 || id.changed_nanos >= 1_000_000_000 {
            return Err(invalid("invalid launch file identity"));
        }
        Ok(id)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Cohort {
    pub negative: bool,
    pub hardware: bool,
    pub session: u64,
    pub completions: u64,
    pub owner_proof: u8,
    pub worker: Option<FileIdentity>,
    pub driver: Option<FileIdentity>,
    pub last_job: Option<JobIdentity>,
}

impl Capture {
    pub fn write(&self, w: &mut impl Write, cohort: Cohort, complete: bool) -> io::Result<()> {
        w.write_all(HEADER)?;
        w.write_all(&[
            u8::from(cohort.negative),
            u8::from(cohort.hardware),
            cohort.owner_proof,
        ])?;
        w.number(cohort.session)?;
        w.number(cohort.completions)?;
        w.number(self.events.len() as u64)?;
        w.number(self.samples.len() as u64)?;
        w.optional(cohort.worker, |w, id| id.write(w))?;
        w.optional(cohort.driver, |w, id| id.write(w))?;
        w.optional(cohort.last_job, |w, job| w.job(job))?;
        for event in &self.events {
            event.write(w)?;
        }
        for (sample, bytes) in self
            .samples
            .iter()
            .zip(self.data.as_chunks::<SAMPLE_BYTES>().0)
        {
            w.job(sample.job)?;
            w.optional(sample.bounds, |w, bounds| w.rect(bounds))?;
            w.optional(sample.anchor, |w, anchor| w.point(anchor))?;
            w.write_all(&bytes[..IMAGE + MASK])?;
            w.optional(sample.readback, |w, readback| {
                readback.write(w)?;
                w.number(sample.rectangles as u64)?;
                w.write_all(&bytes[IMAGE + MASK..2 * IMAGE + MASK + sample.rectangles * 8])
            })?;
        }
        if complete && self.available() && !self.pending {
            w.write_all(FOOTER)?;
        } else {
            w.write_all(b"ROOMIE-INCOMPLETE\x01")?;
            w.write_all(
                self.failure
                    .unwrap_or("owner or worker evidence unsuccessful")
                    .as_bytes(),
            )?;
        }
        w.flush()
    }

    pub fn failure(&self) -> Option<&'static str> {
        self.failure.or(self
            .pending
            .then_some("selected commitment lacks observations"))
    }
}

#[derive(Default, Debug)]
pub(crate) struct Counts {
    pub jobs: u64,
    pub dispositions: u64,
    pub completions: u64,
    pub valid: u64,
    pub attempts: u64,
    pub commitments: u64,
    pub observed: u64,
}

struct Job {
    identity: JobIdentity,
    request: FrameRequest,
    deadline: Duration,
    state: Kind,
    disposition: Option<Kind>,
    canceled: bool,
    expired: bool,
    committed_at: Option<Duration>,
}

struct Trace {
    current: Option<Job>,
    startup: bool,
    shutdown: bool,
    at: Duration,
    counts: Counts,
    observations: Vec<(JobIdentity, FrameRequest, Duration, Duration)>,
}

impl Trace {
    fn new() -> io::Result<Self> {
        let mut observations = Vec::new();
        observations
            .try_reserve_exact(SAMPLES)
            .map_err(|_| invalid("could not allocate observation index"))?;
        Ok(Self {
            current: None,
            startup: false,
            shutdown: false,
            at: Duration::ZERO,
            counts: Counts::default(),
            observations,
        })
    }

    fn settled(&self) -> bool {
        self.current.as_ref().is_none_or(|job| match job.state {
            Kind::D1Obsolete
            | Kind::Obsolete
            | Kind::Withdrawn
            | Kind::Rejected
            | Kind::Skipped
            | Kind::Observed
            | Kind::CanceledReady
            | Kind::Stale => true,
            Kind::Expired => job.disposition == Some(Kind::Frame),
            _ => false,
        })
    }

    fn accept(&mut self, event: Event, session: u64) -> io::Result<()> {
        use Kind::*;
        let bad = || invalid("contradictory owner event ordering, identity, time or disposition");
        if let Some(at) = event.at {
            if at < self.at {
                return Err(bad());
            }
            self.at = at;
        }
        if event.kind != Admit && event.admission.is_some() {
            return Err(bad());
        }
        match event.kind {
            Startup
                if event.job.is_none() && event.at.is_none() && !self.startup && !self.shutdown =>
            {
                self.startup = true;
                return Ok(());
            }
            Expose if event.job.is_none() && event.at.is_none() && self.startup => return Ok(()),
            Shutdown
                if event.job.is_none() && event.at.is_some() && self.startup && !self.shutdown =>
            {
                self.shutdown = true;
                return Ok(());
            }
            _ => {}
        }
        let identity = event.job.ok_or_else(bad)?;
        if identity.session != session {
            return Err(bad());
        }
        if event.kind == Admit {
            let (request, deadline) = event.admission.ok_or_else(bad)?;
            if !self.startup
                || self.shutdown
                || !self.settled()
                || event
                    .at
                    .and_then(|at| at.checked_add(Duration::from_millis(100)))
                    != Some(deadline)
                || identity.serial != self.counts.jobs + 1
                || identity.frame.sequence == 0
                || request.identity() != identity.frame
                || request.extent().size() != Size::new(SIDE as u32, SIDE as u32)
                || self.current.as_ref().is_some_and(|job| {
                    job.identity.epoch > identity.epoch
                        || job.request.active_elapsed() > request.active_elapsed()
                })
            {
                return Err(bad());
            }
            self.counts.jobs += 1;
            self.current = Some(Job {
                identity,
                request,
                deadline,
                state: Admit,
                disposition: None,
                canceled: false,
                expired: false,
                committed_at: None,
            });
            return Ok(());
        }
        let job = self
            .current
            .as_mut()
            .filter(|job| job.identity == identity)
            .ok_or_else(bad)?;
        match event.kind {
            Frame | Obsolete | Withdrawn | Rejected
                if job.disposition.is_none()
                    && matches!(job.state, Admit | CanceledPending | Expired)
                    && event.at.is_some() =>
            {
                job.disposition = Some(event.kind);
                self.counts.dispositions += 1;
                if matches!(event.kind, Frame | Obsolete) {
                    self.counts.completions += 1;
                }
                if event.kind == Frame {
                    self.counts.valid += 1;
                }
            }
            CanceledPending
                if job.disposition.is_none()
                    && !job.canceled
                    && matches!(job.state, Admit | Expired)
                    && event.at.is_some() =>
            {
                job.canceled = true
            }
            Expired
                if !job.expired
                    && matches!(job.state, Admit | CanceledPending | Ready)
                    && event.at.is_some_and(|at| at >= job.deadline) =>
            {
                job.expired = true
            }
            Ready
                if job.state == Frame
                    && !job.canceled
                    && !job.expired
                    && event.at.is_some_and(|at| at < job.deadline) => {}
            CanceledReady | Stale if job.state == Ready && event.at.is_some() => {}
            Selected if job.state == Ready && event.at.is_none() && !self.shutdown => {}
            Refused if job.state == Selected && event.at.is_some_and(|at| at < job.deadline) => {}
            RefusedExpired
                if job.state == Selected && event.at.is_some_and(|at| at >= job.deadline) => {}
            D1Obsolete
                if job.state == Frame && (job.canceled || job.expired) && event.at.is_some() => {}
            Admitted if job.state == Selected && event.at.is_some_and(|at| at < job.deadline) => {
                self.counts.attempts += 1
            }
            Skipped
                if matches!(job.state, Selected | Refused | RefusedExpired)
                    && event.at.is_none() => {}
            Committed if job.state == Admitted && event.at.is_some() => {
                self.counts.commitments += 1;
                job.committed_at = event.at;
            }
            Observed
                if job.state == Committed
                    && event.at.is_some()
                    && self.observations.len() < SAMPLES =>
            {
                self.counts.observed += 1;
                self.observations.push((
                    identity,
                    job.request,
                    job.committed_at.ok_or_else(bad)?,
                    event.at.ok_or_else(bad)?,
                ));
            }
            _ => return Err(bad()),
        }
        job.state = event.kind;
        Ok(())
    }
}

pub(crate) fn verify(
    mut r: impl Read,
    telemetry: impl FnOnce(Cohort) -> std::result::Result<(), String>,
) -> std::result::Result<String, String> {
    verify_inner(&mut r, telemetry).map_err(|e| e.to_string())
}

fn verify_inner(
    r: &mut impl Decode,
    telemetry: impl FnOnce(Cohort) -> std::result::Result<(), String>,
) -> io::Result<String> {
    let mut header = [0; HEADER.len()];
    r.read_exact(&mut header)?;
    if header != HEADER {
        return Err(invalid("unknown evidence format or version"));
    }
    let [negative, hardware, owner_proof] = r.bytes::<3>()?;
    if negative > 1 || hardware > 1 || owner_proof != if negative == 1 { 255 } else { 127 } {
        return Err(invalid("unknown cohort mode or adapter"));
    }
    let session = r.number()?;
    let completions = r.number()?;
    let events: usize = r.number()?;
    let samples: usize = r.number()?;
    if events > EVENTS || samples > SAMPLES {
        return Err(invalid("evidence counts exceed limits"));
    }
    let cohort = Cohort {
        negative: negative != 0,
        hardware: hardware != 0,
        owner_proof,
        session,
        completions,
        worker: r.optional(FileIdentity::read)?,
        driver: r.optional(FileIdentity::read)?,
        last_job: r.optional(|r| r.job())?,
    };
    let worker = cohort
        .worker
        .ok_or_else(|| invalid("missing worker identity"))?;
    let driver = cohort
        .driver
        .ok_or_else(|| invalid("missing ICD manifest identity"))?;
    let mut trace = Trace::new()?;
    for _ in 0..events {
        trace.accept(Event::read(r)?, cohort.session)?;
    }
    let c = &trace.counts;
    if !trace.startup
        || !trace.shutdown
        || !trace.settled()
        || c.dispositions != c.jobs
        || c.completions != cohort.completions
        || c.commitments != c.attempts
        || c.observed != c.commitments
        || trace.observations.len() != samples
        || trace.current.as_ref().map(|job| job.identity) != cohort.last_job
        || (cohort.negative && (c.jobs != 0 || samples != 0))
    {
        return Err(invalid(
            "incomplete or contradictory owner/worker reconciliation",
        ));
    }
    let mut phases = Phases::default();
    let mut previous = None;
    for &(job, request, committed_at, observed_at) in &trace.observations {
        if previous.is_some_and(|(prior, at): (FrameIdentity, Duration)| {
            job.frame.generation < prior.generation
                || (job.frame.generation == prior.generation
                    && prior.sequence.checked_add(1) != Some(job.frame.sequence))
                || committed_at < at
        }) {
            return Err(invalid("nonadjacent or duplicate committed frame"));
        }
        verify_sample(r, job, request, committed_at, observed_at)?;
        phases.add(request);
        previous = Some((job.frame, committed_at));
    }
    let mut footer = [0; FOOTER.len()];
    r.read_exact(&mut footer)?;
    let mut extra = [0];
    if footer != FOOTER || r.read(&mut extra)? != 0 {
        return Err(invalid("evidence lacks exact completeness footer"));
    }
    telemetry(cohort).map_err(io::Error::other)?;
    if cohort.negative {
        Ok(format!(
            "Expected Vulkan initialization rejection verified. Worker {worker:?}. ICD {driver:?}."
        ))
    } else {
        phases.finish().map_err(invalid)?;
        Ok(format!(
            "Exact managed Vulkan correctness verified for {samples} commitments. {c:?}. Worker {worker:?}. ICD {driver:?}."
        ))
    }
}

#[derive(Default)]
struct Phases {
    prior: Option<(u64, u128)>,
    cycle: Option<u128>,
    complete: bool,
}
impl Phases {
    fn add(&mut self, request: FrameRequest) {
        let interval = request.active_elapsed().as_millis() / 300;
        let generation = request.identity().generation;
        if self.prior.is_some_and(|(previous_generation, previous)| {
            previous_generation != generation || interval < previous || interval - previous > 1
        }) {
            self.cycle = None;
        }
        if let Some(first) = self.cycle {
            self.complete |= interval - first == 5;
        }
        if interval.is_multiple_of(5) {
            self.cycle = Some(interval);
        }
        self.prior = Some((generation, interval));
    }

    fn finish(&self) -> Result<()> {
        if !self.complete {
            return Err("missing actual ordered, adjacent GPU phase cycle");
        }
        Ok(())
    }
}

fn verify_sample(
    r: &mut impl Decode,
    job: JobIdentity,
    request: FrameRequest,
    committed_at: Duration,
    observed_at: Duration,
) -> io::Result<()> {
    if r.job()? != job {
        return Err(invalid("sample and owner commitment identity differ"));
    }
    let bounds = r.optional(|r| r.rect())?;
    let anchor = r.optional(|r| r.point())?;
    let expected = SyntheticProducer::new()
        .prepare_frame(&request)
        .map_err(|_| invalid("CPU oracle rejected original request"))?;
    compare_pixels(r, &expected.payload)?;
    let mask = metadata_mask(&expected.metadata).map_err(invalid)?;
    if bounds != expected.metadata.visible_bounds()
        || anchor != expected.metadata.anchor()
        || r.bytes::<MASK>()? != mask
    {
        return Err(invalid(
            "candidate bounds, anchor or coverage differ from CPU oracle",
        ));
    }
    r.optional(|r| {
        let readback = Readback::read(r)?;
        readback.layout.validate(IMAGE).map_err(invalid)?;
        if readback.target.identity != job.frame
            || readback.image_at < committed_at
            || readback.shape_at != observed_at
        {
            return Err(invalid("readback does not belong to the commitment"));
        }
        let count: usize = r.number()?;
        if count > RECTANGLES {
            return Err(invalid("oversized native rectangle count"));
        }
        readback.validate(count).map_err(invalid)?;
        compare_pixels(r, &expected.payload)?;
        let mut coverage = RectangleUnion::new();
        for _ in 0..count {
            let [a, b, c, d, e, f, g, h] = r.bytes()?;
            coverage
                .add(Rectangle {
                    x: i16::from_le_bytes([a, b]),
                    y: i16::from_le_bytes([c, d]),
                    width: u16::from_le_bytes([e, f]),
                    height: u16::from_le_bytes([g, h]),
                })
                .map_err(invalid)?;
        }
        if coverage.mask() != mask {
            return Err(invalid("native SHAPE differs from CPU coverage"));
        }
        Ok(())
    })?
    .ok_or_else(|| invalid("commitment is missing native replies"))
}

fn compare_pixels(r: &mut impl Decode, expected: &[u8]) -> io::Result<()> {
    for row in expected.as_chunks::<{ SIDE * 4 }>().0 {
        if &r.bytes::<{ SIDE * 4 }>()? != row {
            return Err(invalid("payload differs from exact CPU BGRA oracle"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "probe_tests/managed_vulkan_observer_tests.rs"]
mod managed_vulkan_observer_tests;
