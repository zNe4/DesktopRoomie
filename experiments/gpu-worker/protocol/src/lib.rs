//! D2-A's synthetic-only, bounded, little-endian worker protocol.
//! See ../../WIRE.md for the byte contract and settlement assertions.
use std::io::{self, Read, Write};
use std::time::Duration;

pub const VERSION: u16 = 1;
pub const WIDTH: u16 = 160;
pub const HEIGHT: u16 = 160;
pub const FRAME_BYTES: usize = 102_400;
pub const MAX_DIAGNOSTIC: usize = 512;
pub const HEADER_BYTES: usize = 24;
pub const MAX_BODY: usize = 32 + 4 + FRAME_BYTES;
pub const MAX_MESSAGE: usize = HEADER_BYTES + MAX_BODY;
const MAGIC: &[u8; 4] = b"DRWK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobKey {
    pub epoch: u64,
    pub serial: u64,
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Idle,
    Walk,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    Neutral,
    FrontRight,
    FrontLeft,
    BackRight,
    BackLeft,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Render {
    pub job: JobKey,
    pub action: Action,
    pub facing: Facing,
    pub elapsed: Duration,
    pub delta: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Initialize,
    Ready,
    Render(Render),
    Frame { job: JobKey, bgra: Vec<u8> },
    RejectedBeforeSubmission { job: JobKey, diagnostic: String },
    JobFailed { job: JobKey, diagnostic: String },
    Unresolved { job: JobKey, diagnostic: String },
    BackendFault(String),
    Shutdown,
    Stopped,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub session: u64,
    pub body: Body,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Header,
    Version,
    Tag,
    Length,
    Enum,
    Numeric,
    Utf8,
    Truncated,
    Backpressure,
    Poisoned,
}

fn tag(body: &Body) -> u16 {
    match body {
        Body::Initialize => 1,
        Body::Ready => 2,
        Body::Render(_) => 3,
        Body::Frame { .. } => 4,
        Body::RejectedBeforeSubmission { .. } => 5,
        Body::JobFailed { .. } => 6,
        Body::Unresolved { .. } => 7,
        Body::BackendFault(_) => 8,
        Body::Shutdown => 9,
        Body::Stopped => 10,
    }
}
fn key(out: &mut Vec<u8>, job: JobKey) -> Result<(), Error> {
    if job.serial == 0 {
        return Err(Error::Numeric);
    }
    for n in [job.epoch, job.serial, job.generation, job.sequence] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    Ok(())
}
fn diagnostic(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    if value.len() > MAX_DIAGNOSTIC {
        return Err(Error::Length);
    }
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
fn duration(out: &mut Vec<u8>, value: Duration) {
    out.extend_from_slice(&value.as_secs().to_le_bytes());
    out.extend_from_slice(&value.subsec_nanos().to_le_bytes());
}
pub fn encode(message: &Message) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(HEADER_BYTES);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&tag(&message.body).to_le_bytes());
    out.extend_from_slice(&[0; 8]); // body length and reserved
    out.extend_from_slice(&message.session.to_le_bytes());
    match &message.body {
        Body::Initialize => {
            out.extend_from_slice(&WIDTH.to_le_bytes());
            out.extend_from_slice(&HEIGHT.to_le_bytes());
            out.push(1); // tightly packed top-left premultiplied BGRA8
        }
        Body::Ready | Body::Shutdown | Body::Stopped => {}
        Body::Render(render) => {
            if render.delta > render.elapsed {
                return Err(Error::Numeric);
            }
            key(&mut out, render.job)?;
            out.extend_from_slice(&WIDTH.to_le_bytes());
            out.extend_from_slice(&HEIGHT.to_le_bytes());
            out.push(match render.action {
                Action::Idle => 0,
                Action::Walk => 1,
            });
            out.push(match render.facing {
                Facing::Neutral => 0,
                Facing::FrontRight => 1,
                Facing::FrontLeft => 2,
                Facing::BackRight => 3,
                Facing::BackLeft => 4,
            });
            duration(&mut out, render.elapsed);
            duration(&mut out, render.delta);
        }
        Body::Frame { job, bgra } => {
            if bgra.len() != FRAME_BYTES {
                return Err(Error::Length);
            }
            key(&mut out, *job)?;
            out.extend_from_slice(&(FRAME_BYTES as u32).to_le_bytes());
            out.extend_from_slice(bgra);
        }
        Body::RejectedBeforeSubmission {
            job,
            diagnostic: value,
        }
        | Body::JobFailed {
            job,
            diagnostic: value,
        }
        | Body::Unresolved {
            job,
            diagnostic: value,
        } => {
            key(&mut out, *job)?;
            diagnostic(&mut out, value)?;
        }
        Body::BackendFault(value) => diagnostic(&mut out, value)?,
    }
    let len = (out.len() - HEADER_BYTES) as u32;
    out[8..12].copy_from_slice(&len.to_le_bytes());
    Ok(out)
}

struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let bytes = self.0.get(..N).ok_or(Error::Length)?;
        let value = bytes.try_into().map_err(|_| Error::Length)?;
        self.0 = &self.0[N..];
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn key(&mut self) -> Result<JobKey, Error> {
        let job = JobKey {
            epoch: self.u64()?,
            serial: self.u64()?,
            generation: self.u64()?,
            sequence: self.u64()?,
        };
        if job.serial == 0 {
            return Err(Error::Numeric);
        }
        Ok(job)
    }
    fn extent(&mut self) -> Result<(), Error> {
        if self.u16()? != WIDTH || self.u16()? != HEIGHT {
            return Err(Error::Numeric);
        }
        Ok(())
    }
    fn duration(&mut self) -> Result<Duration, Error> {
        let seconds = self.u64()?;
        let nanos = self.u32()?;
        if nanos >= 1_000_000_000 {
            return Err(Error::Numeric);
        }
        Ok(Duration::new(seconds, nanos))
    }
    fn diagnostic(&mut self) -> Result<String, Error> {
        let len = usize::from(self.u16()?);
        if len > MAX_DIAGNOSTIC || self.0.len() != len {
            return Err(Error::Length);
        }
        let text = std::str::from_utf8(self.0)
            .map_err(|_| Error::Utf8)?
            .to_owned();
        self.0 = &[];
        Ok(text)
    }
}

fn header(bytes: &[u8]) -> Result<(u16, usize, u64), Error> {
    let mut cursor = Cursor(bytes);
    if &cursor.take::<4>()? != MAGIC {
        return Err(Error::Header);
    }
    if cursor.u16()? != VERSION {
        return Err(Error::Version);
    }
    let tag = cursor.u16()?;
    let len = usize::try_from(cursor.u32()?).map_err(|_| Error::Length)?;
    if cursor.u32()? != 0 {
        return Err(Error::Header);
    }
    let session = cursor.u64()?;
    let valid = match tag {
        1 => len == 5,
        2 | 9 | 10 => len == 0,
        3 => len == 62,
        4 => len == MAX_BODY,
        5..=7 => (34..=34 + MAX_DIAGNOSTIC).contains(&len),
        8 => (2..=2 + MAX_DIAGNOSTIC).contains(&len),
        _ => return Err(Error::Tag),
    };
    if !valid {
        return Err(Error::Length);
    }
    Ok((tag, len, session))
}
fn decode(bytes: &[u8]) -> Result<Message, Error> {
    let (tag, len, session) = header(bytes)?;
    if bytes.len() != HEADER_BYTES + len {
        return Err(Error::Length);
    }
    let mut c = Cursor(&bytes[HEADER_BYTES..]);
    let body = match tag {
        1 => {
            c.extent()?;
            if c.take::<1>()? != [1] {
                return Err(Error::Enum);
            }
            Body::Initialize
        }
        2 => Body::Ready,
        3 => {
            let job = c.key()?;
            c.extent()?;
            let action = match c.take::<1>()?[0] {
                0 => Action::Idle,
                1 => Action::Walk,
                _ => return Err(Error::Enum),
            };
            let facing = match c.take::<1>()?[0] {
                0 => Facing::Neutral,
                1 => Facing::FrontRight,
                2 => Facing::FrontLeft,
                3 => Facing::BackRight,
                4 => Facing::BackLeft,
                _ => return Err(Error::Enum),
            };
            let elapsed = c.duration()?;
            let delta = c.duration()?;
            if delta > elapsed {
                return Err(Error::Numeric);
            }
            Body::Render(Render {
                job,
                action,
                facing,
                elapsed,
                delta,
            })
        }
        4 => {
            let job = c.key()?;
            if usize::try_from(c.u32()?).map_err(|_| Error::Length)? != FRAME_BYTES {
                return Err(Error::Length);
            }
            let bgra = c.0.to_vec();
            c.0 = &[];
            Body::Frame { job, bgra }
        }
        5..=7 => {
            let job = c.key()?;
            let diagnostic = c.diagnostic()?;
            match tag {
                5 => Body::RejectedBeforeSubmission { job, diagnostic },
                6 => Body::JobFailed { job, diagnostic },
                _ => Body::Unresolved { job, diagnostic },
            }
        }
        8 => Body::BackendFault(c.diagnostic()?),
        9 => Body::Shutdown,
        10 => Body::Stopped,
        _ => return Err(Error::Tag),
    };
    if !c.0.is_empty() {
        return Err(Error::Length);
    }
    Ok(Message { session, body })
}

/// One incremental frame, with no attacker-sized allocation. `feed` consumes at
/// most one message; the caller retains any following bytes and drains `take`.
pub struct Decoder {
    bytes: Vec<u8>,
    target: usize,
    poisoned: bool,
}
impl Default for Decoder {
    fn default() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_MESSAGE),
            target: HEADER_BYTES,
            poisoned: false,
        }
    }
}
impl Decoder {
    pub fn buffered(&self) -> usize {
        self.bytes.len()
    }
    pub fn remaining(&self) -> usize {
        self.target - self.bytes.len()
    }
    pub fn feed(&mut self, input: &[u8]) -> Result<usize, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.remaining() == 0 {
            return Err(Error::Backpressure);
        }
        let count = input.len().min(self.remaining());
        self.bytes.extend_from_slice(&input[..count]);
        if self.bytes.len() == HEADER_BYTES {
            match header(&self.bytes) {
                Ok((_, len, _)) => self.target = HEADER_BYTES + len,
                Err(error) => {
                    self.poisoned = true;
                    return Err(error);
                }
            }
        }
        Ok(count)
    }
    pub fn take(&mut self) -> Result<Option<Message>, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.remaining() != 0 {
            return Ok(None);
        }
        let result = decode(&self.bytes);
        self.poisoned = result.is_err();
        self.bytes.clear();
        self.target = HEADER_BYTES;
        result.map(Some)
    }
    pub fn eof(&self) -> Result<(), Error> {
        if self.poisoned {
            Err(Error::Poisoned)
        } else if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(Error::Truncated)
        }
    }
}

/// One bounded transmit slot. Backpressure never overwrites a partial message.
#[derive(Default)]
pub struct Transmitter {
    bytes: Vec<u8>,
    offset: usize,
}
impl Transmitter {
    pub fn pending(&self) -> usize {
        self.bytes.len() - self.offset
    }
    pub fn queue(&mut self, message: &Message) -> Result<(), Error> {
        if self.pending() != 0 {
            return Err(Error::Backpressure);
        }
        self.bytes = encode(message)?;
        self.offset = 0;
        Ok(())
    }
    /// At most one syscall per turn. Interrupted/WouldBlock retains all bytes.
    pub fn write(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if self.pending() == 0 {
            return Ok(());
        }
        match writer.write(&self.bytes[self.offset..]) {
            Ok(0) => Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => {
                self.offset += count;
                Ok(())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

/// Reads only the current frame boundary, leaving coalesced messages in the
/// socket. At most one syscall, bounded by a small stack buffer.
pub fn read_step(decoder: &mut Decoder, reader: &mut impl Read) -> io::Result<Option<Message>> {
    if let Some(message) = decoder.take().map_err(invalid)? {
        return Ok(Some(message));
    }
    let mut buffer = [0; 8192];
    let limit = buffer.len().min(decoder.remaining());
    match reader.read(&mut buffer[..limit]) {
        Ok(0) => {
            decoder.eof().map_err(invalid)?;
            Err(io::ErrorKind::UnexpectedEof.into())
        }
        Ok(count) => {
            decoder.feed(&buffer[..count]).map_err(invalid)?;
            decoder.take().map_err(invalid)
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
fn invalid(error: Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}"))
}

#[cfg(test)]
mod tests;
