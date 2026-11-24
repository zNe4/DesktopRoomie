use desktop_probe::presentation::{self as p, gpu_handoff::*};
use roomie_worker_acceptance::{
    oracle,
    scheduler::{self, Samples, Scheduler},
};
use std::time::{Duration, Instant};

#[test]
fn oracle_rejects_pixel_alpha_identity_and_hit_geometry_corruption() {
    for ms in oracle::SAMPLE_MS {
        let r = oracle::request(19, Duration::from_millis(ms));
        let mut frame = oracle::expected(&r).unwrap();
        oracle::validate(&r, &frame).unwrap();
        frame.payload[0] = 1;
        assert!(oracle::validate(&r, &frame).is_err());
        frame.payload[0] = 0;
        frame.payload[3] = 1;
        assert!(oracle::validate(&r, &frame).is_err());
        frame.payload[3] = 0;
        frame.identity.sequence += 1;
        assert!(oracle::validate(&r, &frame).is_err());
        frame.identity = r.identity();
        frame.metadata = p::FrameMetadata::new(
            r.extent(),
            None,
            None,
            p::HitRegion::new(
                r.extent(),
                [p::RowSpan {
                    y: 0,
                    start_x: 0,
                    end_x: 1,
                }],
            )
            .unwrap(),
        )
        .unwrap();
        assert!(oracle::validate(&r, &frame).is_err());
    }
}
#[test]
fn oracle_rejects_channel_swap_row_inversion_and_wrong_byte_count() {
    let r = oracle::request(1, Duration::from_millis(900));
    let cpu = oracle::expected(&r).unwrap();
    for altered in [
        cpu.payload
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect::<Vec<_>>(),
        cpu.payload
            .as_chunks::<640>()
            .0
            .iter()
            .rev()
            .flatten()
            .copied()
            .collect(),
        vec![0; 102399],
        vec![0; 102401],
    ] {
        let frame = p::PreparedFrame {
            identity: r.identity(),
            metadata: cpu.metadata.clone(),
            payload: altered,
        };
        assert!(oracle::validate(&r, &frame).is_err());
    }
}
#[test]
fn fixture_has_exact_accepted_schedule_geometry_and_phase_wrap() {
    let counts = [3136, 3136, 2303, 1446, 0];
    for (i, ms) in oracle::SAMPLE_MS.into_iter().enumerate() {
        let expected = oracle::expected(&oracle::request(1, Duration::from_millis(ms))).unwrap();
        assert_eq!(
            expected
                .payload
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] != 0)
                .count(),
            counts[i]
        );
        for offset in [0, 299] {
            let other = oracle::expected(&oracle::request(
                1,
                Duration::from_millis(ms + offset + 1500),
            ))
            .unwrap();
            assert_eq!(expected.payload, other.payload);
            assert_eq!(expected.metadata, other.metadata);
        }
    }
}
#[test]
fn explicit_settlement_without_payload_never_creates_a_ready_frame() {
    let now = Instant::now();
    let mut owner: GpuHandoff<Vec<u8>> = GpuHandoff::new(41);
    owner.resume().unwrap();
    let job = owner
        .schedule(
            oracle::request(1, Duration::ZERO),
            now,
            now + Duration::from_secs(1),
        )
        .unwrap();
    let mut wrong = job.identity();
    wrong.session += 1;
    assert_eq!(
        owner.complete(wrong, JobCompletion::SettledWithoutPayload, now),
        Err(CompletionError::UnexpectedJob)
    );
    assert_eq!(owner.outstanding(), Some(job.identity()));
    assert_eq!(
        owner.complete(job.identity(), JobCompletion::SettledWithoutPayload, now),
        Ok(CompletionStatus::Obsolete)
    );
    assert!(owner.take_ready().is_none());
    assert!(owner.outstanding().is_none());
    assert_eq!(
        owner.complete(job.identity(), JobCompletion::SettledWithoutPayload, now),
        Err(CompletionError::UnexpectedJob)
    );
}
#[test]
fn settlement_after_cancel_resume_or_shutdown_discharges_only_the_exact_job() {
    for shutdown in [false, true] {
        let now = Instant::now();
        let mut owner: GpuHandoff<Vec<u8>> = GpuHandoff::new(1);
        owner.resume().unwrap();
        let job = owner
            .schedule(
                oracle::request(1, Duration::ZERO),
                now,
                now + Duration::from_secs(1),
            )
            .unwrap();
        if shutdown {
            owner.shutdown();
        } else {
            owner.interrupt().unwrap();
            owner.resume().unwrap();
        }
        assert!(owner
            .schedule(
                oracle::request(2, Duration::ZERO),
                now,
                now + Duration::from_secs(1)
            )
            .is_err());
        assert_eq!(
            owner.complete(job.identity(), JobCompletion::SettledWithoutPayload, now),
            Ok(CompletionStatus::Obsolete)
        );
        assert!(owner.outstanding().is_none());
        assert!(owner.take_ready().is_none());
    }
}
#[test]
fn rational_scheduler_has_exact_warmup_and_measurement_boundaries() {
    let mut scheduler = Scheduler::default();
    for index in 0..720 {
        let at = scheduler::deadline(index);
        if index > 0 {
            assert!(scheduler
                .poll(at - Duration::from_nanos(1))
                .unwrap()
                .is_none());
        }
        let op = scheduler.poll(at).unwrap().unwrap();
        assert_eq!(op.index, index);
        assert_eq!(op.lateness, Duration::ZERO);
        assert!(scheduler.poll(at).unwrap().is_none());
    }
    assert_eq!(scheduler::deadline(120), Duration::from_secs(2));
    assert_eq!(scheduler.missed, [0, 0]);
    assert!(scheduler.next_deadline().is_none());
}
#[test]
fn overruns_skip_old_deadlines_without_catch_up_bursts() {
    let mut scheduler = Scheduler::default();
    assert_eq!(scheduler.poll(Duration::ZERO).unwrap().unwrap().index, 0);
    let op = scheduler.poll(Duration::from_millis(91)).unwrap().unwrap();
    assert_eq!(op.index, 5);
    assert_eq!(scheduler.missed, [4, 0]);
    assert!(scheduler.poll(Duration::from_millis(91)).unwrap().is_none());
    assert_eq!(
        scheduler.next_deadline(),
        Some(Duration::from_millis(91) + scheduler::deadline(6) - scheduler::deadline(5))
    );
}
#[test]
fn overrun_crossing_warmup_splits_misses_correctly() {
    let mut scheduler = Scheduler::default();
    let op = scheduler
        .poll(Duration::from_millis(2051))
        .unwrap()
        .unwrap();
    assert_eq!(op.index, 123);
    assert_eq!(scheduler.missed, [120, 3]);
    assert_eq!(scheduler::segment(op.index), 1);
    assert!(scheduler
        .poll(Duration::from_millis(2051))
        .unwrap()
        .is_none());
}
#[test]
fn scheduler_end_cancel_regression_and_arbitrarily_large_delay_are_bounded() {
    let mut scheduler = Scheduler::default();
    assert!(scheduler.poll(Duration::MAX).unwrap().is_none());
    assert_eq!(scheduler.missed, [120, 600]);
    let mut scheduler = Scheduler::default();
    scheduler.poll(Duration::from_secs(2)).unwrap();
    scheduler.cancel();
    assert_eq!(scheduler.missed, [120, 0]);
    assert_eq!(scheduler.skipped, [0, 599]);
    assert!(scheduler.poll(Duration::from_secs(3)).unwrap().is_none());
    assert!(scheduler.poll(Duration::ZERO).is_err());
}
#[test]
fn controlled_time_stress_with_processing_delays_keeps_one_outstanding() {
    for service in [Duration::from_millis(1), Duration::from_millis(42)] {
        let mut scheduler = Scheduler::default();
        let mut now = Duration::ZERO;
        let mut completion = None;
        let mut attempts = [0_u64; 2];
        let mut busy = [0_u64; 2];
        let mut admitted = [0_u64; 2];
        while let Some(deadline) = scheduler.next_deadline() {
            now = now.max(deadline);
            if completion.is_some_and(|d| now >= d) {
                completion = None;
            }
            if let Some(op) = scheduler.poll(now).unwrap() {
                let group = scheduler::segment(op.index);
                attempts[group] += 1;
                if completion.is_some() {
                    busy[group] += 1;
                } else {
                    admitted[group] += 1;
                    completion = Some(now + service);
                }
            }
            // A controlled owner overrun, deliberately crossing a phase and
            // warmup transition. The next wakeup emits one latest request.
            if now == Duration::from_millis(1900) {
                now += Duration::from_millis(171);
            }
        }
        for i in 0..2 {
            assert_eq!(attempts[i], busy[i] + admitted[i]);
            assert_eq!(attempts[i] + scheduler.missed[i], [120, 600][i]);
        }
        if service > Duration::from_millis(17) {
            assert!(busy[1] > 0);
        } else {
            assert_eq!(busy, [0, 0]);
        }
    }
}
#[test]
fn timing_samples_are_bounded_and_percentiles_are_nearest_rank() {
    let mut samples = Samples::default();
    assert_eq!(samples.percentile(95), None);
    for n in 1..=720 {
        samples.push(Duration::from_nanos(n)).unwrap();
    }
    assert_eq!(samples.percentile(50), Some(Duration::from_nanos(360)));
    assert_eq!(samples.percentile(95), Some(Duration::from_nanos(684)));
    assert!(samples.push(Duration::ZERO).is_err());
    assert_eq!(samples.len(), 720);
}
