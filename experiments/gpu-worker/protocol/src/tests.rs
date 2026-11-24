use super::*;
fn job() -> JobKey {
    JobKey {
        epoch: 2,
        serial: 3,
        generation: 4,
        sequence: 5,
    }
}
fn render() -> Render {
    Render {
        job: job(),
        action: Action::Idle,
        facing: Facing::Neutral,
        elapsed: Duration::new(7, 8),
        delta: Duration::new(1, 2),
    }
}
fn message(body: Body) -> Message {
    Message {
        session: 0x0102030405060708,
        body,
    }
}
fn decoded(bytes: &[u8], chunk: usize) -> Result<Vec<Message>, Error> {
    let mut decoder = Decoder::default();
    let mut messages = Vec::new();
    for mut part in bytes.chunks(chunk) {
        while !part.is_empty() {
            let count = decoder.feed(part)?;
            part = &part[count..];
            if let Some(message) = decoder.take()? {
                messages.push(message);
            }
        }
    }
    decoder.eof()?;
    Ok(messages)
}
#[test]
fn every_message_round_trips_and_byte_order_is_fixed() {
    let messages = vec![
        message(Body::Initialize),
        message(Body::Ready),
        message(Body::Render(render())),
        message(Body::Frame {
            job: job(),
            bgra: vec![17; FRAME_BYTES],
        }),
        message(Body::RejectedBeforeSubmission {
            job: job(),
            diagnostic: "rejected".into(),
        }),
        message(Body::JobFailed {
            job: job(),
            diagnostic: "failed".into(),
        }),
        message(Body::Unresolved {
            job: job(),
            diagnostic: "uncertain".into(),
        }),
        message(Body::BackendFault("é".repeat(256))),
        message(Body::Shutdown),
        message(Body::Stopped),
    ];
    for message in messages {
        let bytes = encode(&message).unwrap();
        assert_eq!(
            &bytes[..8],
            b"DRWK\x01\x00"
                .iter()
                .copied()
                .chain(tag(&message.body).to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert_eq!(&bytes[16..24], &[8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(decoded(&bytes, bytes.len()), Ok(vec![message]));
    }
    let bytes = encode(&message(Body::Render(render()))).unwrap();
    assert_eq!(&bytes[8..16], &[62, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[24..32], &2_u64.to_le_bytes());
    assert_eq!(&bytes[60..62], &[0, 0]);
    assert_eq!(&bytes[62..70], &7_u64.to_le_bytes());
    assert_eq!(&bytes[70..74], &8_u32.to_le_bytes());
}
#[test]
fn arbitrary_chunk_boundaries_and_coalesced_messages() {
    let messages = [
        message(Body::Initialize),
        message(Body::Render(render())),
        message(Body::Frame {
            job: job(),
            bgra: vec![0; FRAME_BYTES],
        }),
        message(Body::BackendFault("fault".into())),
    ];
    let bytes: Vec<u8> = messages.iter().flat_map(|m| encode(m).unwrap()).collect();
    for chunk in [1, 2, 3, 7, 23, 24, 25, 85, 8192, MAX_MESSAGE, bytes.len()] {
        assert_eq!(decoded(&bytes, chunk).unwrap(), messages);
    }
}
#[test]
fn headers_reject_oversize_unknown_version_tag_reserved_and_bad_magic_immediately() {
    let valid = encode(&message(Body::Initialize)).unwrap();
    for (offset, replacement, expected) in [
        (0, 0, Error::Header),
        (4, 2, Error::Version),
        (6, 255, Error::Tag),
        (12, 1, Error::Header),
        (8, 255, Error::Length),
    ] {
        let mut bytes = valid[..HEADER_BYTES].to_vec();
        bytes[offset] = replacement;
        let mut decoder = Decoder::default();
        assert_eq!(decoder.feed(&bytes), Err(expected));
        assert!(decoder.buffered() <= HEADER_BYTES);
        assert_eq!(decoder.feed(&[0]), Err(Error::Poisoned));
    }
    let mut bytes = valid[..HEADER_BYTES].to_vec();
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(Decoder::default().feed(&bytes), Err(Error::Length));
}
#[test]
fn frame_length_is_exact_in_header_and_inner_field() {
    for len in [0, FRAME_BYTES - 1, FRAME_BYTES + 1] {
        assert_eq!(
            encode(&message(Body::Frame {
                job: job(),
                bgra: vec![0; len]
            })),
            Err(Error::Length)
        );
    }
    let mut bytes = encode(&message(Body::Frame {
        job: job(),
        bgra: vec![0; FRAME_BYTES],
    }))
    .unwrap();
    assert_eq!(bytes.len(), MAX_MESSAGE);
    bytes[56..60].copy_from_slice(&102399_u32.to_le_bytes());
    assert_eq!(decoded(&bytes, 13), Err(Error::Length));
    bytes[8..12].copy_from_slice(&((MAX_BODY - 1) as u32).to_le_bytes());
    assert_eq!(decoded(&bytes, 13), Err(Error::Length));
}
#[test]
fn numeric_enum_extent_duration_and_utf8_decoding_are_checked() {
    let valid = encode(&message(Body::Render(render()))).unwrap();
    for (offset, replacement, expected) in [
        (60, 2, Error::Enum),
        (61, 5, Error::Enum),
        (56, 0, Error::Numeric),
    ] {
        let mut bytes = valid.clone();
        bytes[offset] = replacement;
        assert_eq!(decoded(&bytes, 1), Err(expected));
    }
    let mut bytes = valid.clone();
    bytes[32..40].fill(0);
    assert_eq!(decoded(&bytes, 1), Err(Error::Numeric));
    bytes = valid.clone();
    bytes[70..74].copy_from_slice(&1_000_000_000_u32.to_le_bytes());
    assert_eq!(decoded(&bytes, 1), Err(Error::Numeric));
    bytes = valid;
    bytes[74..82].copy_from_slice(&8_u64.to_le_bytes());
    assert_eq!(decoded(&bytes, 1), Err(Error::Numeric));
    let mut bytes = encode(&message(Body::BackendFault("x".into()))).unwrap();
    bytes[26] = 255;
    assert_eq!(decoded(&bytes, 1), Err(Error::Utf8));
    assert_eq!(
        encode(&message(Body::BackendFault("x".repeat(513)))),
        Err(Error::Length)
    );
    bytes[24..26].copy_from_slice(&513_u16.to_le_bytes());
    assert_eq!(decoded(&bytes, 1), Err(Error::Length));
}
#[test]
fn eof_at_every_incomplete_boundary_is_truncated() {
    let bytes = encode(&message(Body::Render(render()))).unwrap();
    for len in 1..bytes.len() {
        assert_eq!(decoded(&bytes[..len], 1), Err(Error::Truncated));
    }
    assert!(Decoder::default().eof().is_ok());
}
#[test]
fn receive_slot_is_bounded_and_requires_consumption() {
    let bytes = encode(&message(Body::Frame {
        job: job(),
        bgra: vec![0; FRAME_BYTES],
    }))
    .unwrap();
    let mut decoder = Decoder::default();
    let mut offset = 0;
    while offset != bytes.len() {
        offset += decoder.feed(&bytes[offset..]).unwrap();
    }
    assert_eq!(decoder.buffered(), MAX_MESSAGE);
    assert_eq!(
        decoder.feed(&vec![0; MAX_MESSAGE * 2]),
        Err(Error::Backpressure)
    );
    assert!(decoder.take().unwrap().is_some());
    assert_eq!(decoder.buffered(), 0);
}
struct ShortWriter {
    output: Vec<u8>,
    calls: usize,
    zero: bool,
}
impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.zero {
            return Ok(0);
        }
        match self.calls % 4 {
            0 => Err(io::ErrorKind::WouldBlock.into()),
            1 => Err(io::ErrorKind::Interrupted.into()),
            _ => {
                let count = bytes.len().min(3);
                self.output.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn partial_transmit_and_backpressure_preserve_bytes() {
    let message = message(Body::Render(render()));
    let mut tx = Transmitter::default();
    tx.queue(&message).unwrap();
    let mut writer = ShortWriter {
        output: Vec::new(),
        calls: 0,
        zero: false,
    };
    while tx.pending() != 0 {
        assert_eq!(tx.queue(&message), Err(Error::Backpressure));
        tx.write(&mut writer).unwrap();
        assert!(tx.pending() <= MAX_MESSAGE);
    }
    assert_eq!(writer.output, encode(&message).unwrap());
    tx.queue(&message).unwrap();
    writer.zero = true;
    assert_eq!(
        tx.write(&mut writer).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
}
#[test]
fn read_step_handles_short_reads_interruptions_wouldblock_and_truncated_eof() {
    struct Reader {
        bytes: std::io::Cursor<Vec<u8>>,
        calls: usize,
    }
    impl Read for Reader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.calls += 1;
            match self.calls % 3 {
                0 => Err(io::ErrorKind::WouldBlock.into()),
                1 => Err(io::ErrorKind::Interrupted.into()),
                _ => {
                    let count = out.len().min(2);
                    self.bytes.read(&mut out[..count])
                }
            }
        }
    }
    let messages = [
        message(Body::Ready),
        message(Body::Render(render())),
        message(Body::Shutdown),
    ];
    let bytes: Vec<u8> = messages.iter().flat_map(|m| encode(m).unwrap()).collect();
    let mut reader = Reader {
        bytes: std::io::Cursor::new(bytes.clone()),
        calls: 0,
    };
    let mut decoder = Decoder::default();
    let mut actual = Vec::new();
    loop {
        match read_step(&mut decoder, &mut reader) {
            Ok(Some(message)) => actual.push(message),
            Ok(None) => {}
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
                break;
            }
        }
        assert!(reader.calls < bytes.len() * 4);
    }
    assert_eq!(actual, messages);
    let mut decoder = Decoder::default();
    let mut truncated = std::io::Cursor::new(&bytes[..bytes.len() - 1]);
    loop {
        match read_step(&mut decoder, &mut truncated) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                break;
            }
        }
    }
}
#[test]
fn all_supported_intent_enum_values_round_trip() {
    for action in [Action::Idle, Action::Walk] {
        for facing in [
            Facing::Neutral,
            Facing::FrontRight,
            Facing::FrontLeft,
            Facing::BackRight,
            Facing::BackLeft,
        ] {
            let render = Render {
                action,
                facing,
                ..render()
            };
            let message = message(Body::Render(render));
            assert_eq!(decoded(&encode(&message).unwrap(), 1), Ok(vec![message]));
        }
    }
}
