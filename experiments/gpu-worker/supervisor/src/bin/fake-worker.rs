//! Deterministic protocol fixture, with no GPU, threads or display connection.
use roomie_worker_protocol::{encode, read_step, Body, Decoder, Message, FRAME_BYTES};
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn send(socket: &mut UnixStream, message: Message, fragment: bool) -> io::Result<()> {
    let bytes = encode(&message).expect("synthetic message");
    for chunk in bytes.chunks(if fragment { 7 } else { bytes.len() }) {
        socket.write_all(chunk)?;
    }
    Ok(())
}
/// The test controls race ordering on a separate rendezvous stream. All worker
/// replies and faults still travel through the actual supervised socket.
fn managed_reply(
    socket: &mut UnixStream,
    gate: &mut UnixStream,
    session: u64,
    render: roomie_worker_protocol::Render,
) -> io::Result<()> {
    gate.write_all(b"R")?;
    let mut command = [0];
    gate.read_exact(&mut command)?;
    let mut pixels = vec![0; FRAME_BYTES];
    let phase = (render.elapsed.as_millis() / 300) % 5;
    for y in 0..160 {
        for x in 0..160 {
            let inner = (61..86).contains(&x) && (65..96).contains(&y);
            let pixel = match phase {
                0 if (17..73).contains(&x) && (40..96).contains(&y) => [230, 115, 60, 255],
                1 if (17..73).contains(&x) && (40..96).contains(&y) => [70, 211, 150, 255],
                2 if (51..105).contains(&x) && (53..110).contains(&y) && !inner => {
                    [112, 45, 96, 128]
                }
                3 if (113..137).contains(&x) && (24..58).contains(&y) => [90, 160, 240, 255],
                3 if (132..153).contains(&x) && (101..131).contains(&y) => [35, 95, 120, 128],
                _ => [0; 4],
            };
            let offset = (160 * y + x) * 4;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    if command == *b"M" {
        pixels[0] = 1;
    }
    let mut frame = encode(&Message {
        session,
        body: Body::Frame {
            job: render.job,
            bgra: pixels,
        },
    })
    .unwrap();
    let fault = encode(&Message {
        session,
        body: Body::BackendFault("managed scripted fault".into()),
    })
    .unwrap();
    match command[0] {
        b'A' => frame.extend_from_slice(&fault),
        b'P' => frame.extend_from_slice(&fault[..5]),
        b'G' => frame.extend(fault.repeat(200)),
        b'F' | b'L' | b'M' | b'Q' => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown fixture race",
            ))
        }
    }
    socket.write_all(if command[0] == b'Q' { &fault } else { &frame })?;
    gate.write_all(b"W")?;
    if matches!(command[0], b'P' | b'L' | b'Q') {
        let staged = command[0];
        gate.read_exact(&mut command)?;
        if command[0] != b'C' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown fault release",
            ));
        }
        let suffix = match staged {
            b'P' => &fault[5..],
            b'Q' => &frame,
            _ => &fault,
        };
        socket.write_all(suffix)?;
        gate.write_all(b"W")?;
    }
    Ok(())
}

fn run() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("normal");
    let mut gate = if mode == "managed-script" {
        Some(UnixStream::connect(&args[2])?)
    } else {
        None
    };
    if mode == "fd-check" {
        let fd: i32 = args[2].parse().unwrap();
        // SAFETY: read-only descriptor query; the sentinel must be absent.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1 || std::env::var_os("DISPLAY").is_some()
        {
            std::process::exit(33);
        }
    }
    // SAFETY: the launcher transfers its unique socket endpoint as fd 0.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    let mut decoder = Decoder::default();
    if matches!(mode, "premature-ready" | "pre-init-fault") {
        // Intentionally violate ordering using a session supplied by the test,
        // before reading even one byte of Initialize from the private socket.
        let session = args
            .get(2)
            .and_then(|value| value.parse().ok())
            .unwrap_or(91);
        let body = if mode == "premature-ready" {
            Body::Ready
        } else {
            Body::BackendFault("fault before initialization dispatch".into())
        };
        send(&mut socket, Message { session, body }, false)?;
    }
    let mut initialized = None;
    loop {
        let message = match read_step(&mut decoder, &mut socket) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(error)
                if mode == "parent-death" && error.kind() == io::ErrorKind::UnexpectedEof =>
            {
                // Test PDEATHSIG itself, excluding the cooperative EOF exit race.
                loop {
                    unsafe {
                        libc::pause();
                    }
                }
            }
            Err(error) => return Err(error),
        };
        let session = message.session;
        match message.body {
            Body::Initialize if initialized.is_none() => {
                initialized = Some(session);
                if mode == "exit-init" {
                    return Ok(());
                }
                if matches!(mode, "malformed-header" | "oversized-header") {
                    let mut bytes = encode(&Message {
                        session,
                        body: Body::Ready,
                    })
                    .unwrap();
                    if mode == "malformed-header" {
                        bytes[0] = 0;
                    } else {
                        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
                    }
                    socket.write_all(&bytes)?;
                    continue;
                }
                if mode == "partial-init" {
                    let bytes = encode(&Message {
                        session,
                        body: Body::Ready,
                    })
                    .unwrap();
                    socket.write_all(&bytes[..5])?;
                } else if mode != "stall-init" {
                    send(
                        &mut socket,
                        Message {
                            session,
                            body: Body::Ready,
                        },
                        mode == "fragment",
                    )?;
                }
            }
            Body::Render(render) if initialized == Some(session) => {
                if let Some(gate) = &mut gate {
                    managed_reply(&mut socket, gate, session, render)?;
                    continue;
                }
                if mode == "no-render-expected" {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Render received after admission closed",
                    ));
                }
                let mut job = render.job;
                if mode == "exit-job" {
                    return Ok(());
                }
                if mode == "stall-job" {
                    continue;
                }
                if mode == "wrong-job" {
                    job.sequence ^= 1;
                }
                let body = match mode {
                    "reject" => Body::RejectedBeforeSubmission {
                        job,
                        diagnostic: "fake pre-submit rejection".into(),
                    },
                    "fail" => Body::JobFailed {
                        job,
                        diagnostic: "fake settled failure".into(),
                    },
                    "unresolved" => Body::Unresolved {
                        job,
                        diagnostic: "fake unresolved submission".into(),
                    },
                    _ => {
                        if mode == "late" {
                            std::thread::sleep(Duration::from_millis(150));
                        }
                        let mut bgra = vec![0; FRAME_BYTES];
                        for pixel in bgra.as_chunks_mut::<4>().0 {
                            pixel.copy_from_slice(&[job.serial as u8, 0, 0, 255]);
                        }
                        Body::Frame { job, bgra }
                    }
                };
                let report_session = if mode == "wrong-session" {
                    session ^ 1
                } else {
                    session
                };
                let report = Message {
                    session: report_session,
                    body,
                };
                if mode == "truncated" {
                    let bytes = encode(&report).unwrap();
                    socket.write_all(&bytes[..bytes.len() - 1])?;
                    return Ok(());
                }
                if mode == "duplicate-frame" {
                    let mut bytes = encode(&report).unwrap();
                    bytes.extend_from_within(..);
                    socket.write_all(&bytes)?;
                } else if mode == "flood-faults" {
                    let mut bytes = encode(&report).unwrap();
                    for _ in 0..100 {
                        bytes.extend_from_slice(
                            &encode(&Message {
                                session,
                                body: Body::BackendFault("bounded fault flood".into()),
                            })
                            .unwrap(),
                        );
                    }
                    socket.write_all(&bytes)?;
                } else if matches!(mode, "frame-fault" | "fault-frame") {
                    let fault = Message {
                        session,
                        body: Body::BackendFault("fake backend fault".into()),
                    };
                    let messages = if mode == "frame-fault" {
                        [report, fault]
                    } else {
                        [fault, report]
                    };
                    let bytes: Vec<u8> = messages.iter().flat_map(|m| encode(m).unwrap()).collect();
                    socket.write_all(&bytes)?;
                } else {
                    send(&mut socket, report, mode == "fragment")?;
                }
            }
            Body::Shutdown if initialized == Some(session) => {
                if matches!(
                    mode,
                    "stall-init" | "partial-init" | "stall-job" | "unresolved"
                ) {
                    continue;
                }
                if mode == "no-stop" {
                    return Ok(());
                }
                send(
                    &mut socket,
                    Message {
                        session,
                        body: Body::Stopped,
                    },
                    false,
                )?;
                if mode == "stop-hang" {
                    let _ = socket.read(&mut [0]);
                }
                return Ok(());
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fake protocol violation",
                ))
            }
        }
    }
}
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
