use std::process::{self, ExitCode};
use std::time::{Duration, Instant};

use crate::gpu::{Counters, Gpu, SetupError, WAIT_BUDGET};
use crate::layout::Layout;
use crate::oracle::{self, Oracle};
use crate::readback::{install_native_poll_hook, Identity};
use crate::{PERIOD, PHASES};

const WARMUP: usize = 5;
const MEASURED: usize = 20;
const METRICS: [&str; 8] = [
    "encode",
    "submit",
    "gpu_wait",
    "map",
    "repack",
    "comparison",
    "total",
    "cpu_blocking",
];

pub fn experiment() -> ExitCode {
    install_native_poll_hook();
    println!(
        "experiment=A01-C1-C-B1 rust={:?} wgpu=29.0.1 features=std,vulkan,wgsl eligible_period_ms={} wait_budget_ms={} warmup_sweeps={WARMUP} warmup_frames={} measured_sweeps={MEASURED} measured_frames={}",
        env!("B1_RUSTC"), PERIOD.as_millis(), WAIT_BUDGET.as_millis(), WARMUP * PHASES, MEASURED * PHASES,
    );
    let (layout, oracle, mut gpu) = match initialize() {
        Ok(setup) => setup,
        Err(SetupError::Unavailable(error)) => {
            eprintln!("BLOCKED preflight: {error}");
            report_shutdown(&Counters::default(), "preflight_failed");
            return ExitCode::from(2);
        }
        Err(SetupError::Failed(error)) => {
            eprintln!("FAIL initialization: {error}");
            report_shutdown(&Counters::default(), "initialization_failed");
            return ExitCode::FAILURE;
        }
    };
    let result = benchmark(layout, &oracle, &mut gpu);
    if let Err(error) = &result {
        eprintln!("FAIL {error}");
    }
    if gpu.unsettled {
        report_shutdown(&gpu.counters, "unsettled_submission_process_exit");
        // wgpu-core Queue::drop waits again and can panic if the driver still
        // has not completed the work. This operation created no CPU view.
        // This standalone process is the resource boundary for that failure.
        process::exit(1);
    }
    let counters = std::mem::take(&mut gpu.counters);
    drop(gpu);
    report_shutdown(&counters, "released");
    if result.is_ok() {
        println!("PASS exact five-phase CPU/Vulkan parity");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn initialize() -> Result<(Layout, Oracle, Gpu), SetupError> {
    let layout = Layout::fixture()
        .map_err(|error| SetupError::Failed(format!("fixture layout: {error:?}")))?;
    println!(
        "layout width={} height={} row_bytes={} row_pitch={} staging_bytes={} packed_bytes={}",
        layout.width, layout.height, layout.row, layout.pitch, layout.staging, layout.packed,
    );
    let oracle = Oracle::new(layout).map_err(SetupError::Failed)?;
    let gpu = Gpu::new(layout)?;
    Ok((layout, oracle, gpu))
}

fn benchmark(layout: Layout, oracle: &Oracle, gpu: &mut Gpu) -> Result<(), String> {
    let mut measurements = [const { [[Duration::ZERO; METRICS.len()]; MEASURED] }; PHASES];
    for cycle in 0..WARMUP + MEASURED {
        for (phase, (reference, samples)) in oracle.frames.iter().zip(&mut measurements).enumerate()
        {
            let key = Identity {
                generation: 1,
                phase,
                submission: (cycle * PHASES + phase + 1) as u64,
            };
            let started = Instant::now();
            let (bytes, transfer) = gpu
                .read(key)
                .map_err(|error| format!("{key:?} {error:?}"))?;
            let comparing = Instant::now();
            let coverage = reference.compare(phase, layout, &bytes)?;
            let comparison = comparing.elapsed();
            let values = [
                transfer.encode,
                transfer.submit,
                transfer.gpu_wait,
                transfer.map,
                transfer.repack,
                comparison,
                started.elapsed(),
                transfer.submit + transfer.gpu_wait + transfer.map,
            ];
            if cycle == 0 {
                let alpha_nonzero = coverage.mask.iter().filter(|&&value| value != 0).count();
                println!("parity phase={phase} eligible_ms={} cpu_sha256={} gpu_sha256={} mask_sha256={} covered_pixels={alpha_nonzero} bounds={:?} spans={} differing_bytes=0 differing_pixels=0",
                    (PERIOD * phase as u32).as_millis(), reference.digest, oracle::digest(&bytes),
                    oracle::digest(&coverage.mask), coverage.bounds, coverage.spans.len(),
                );
            }
            let measured = cycle >= WARMUP;
            if measured {
                samples[cycle - WARMUP] = values;
            }
            print!(
                "frame phase={phase} submission={} sample={} cycle={}",
                key.submission,
                if measured { "measured" } else { "warmup" },
                cycle
            );
            for (metric, duration) in METRICS.iter().zip(values) {
                print!(" {metric}_us={:.3}", micros(duration));
            }
            println!();
        }
    }
    for (phase, samples) in measurements.iter().enumerate() {
        for (index, name) in METRICS.iter().enumerate() {
            let mut values = samples.map(|sample| sample[index]);
            values.sort_unstable();
            let nearest_rank = |p: usize| values[(p * values.len()).div_ceil(100) - 1];
            let median = (values[(values.len() - 1) / 2] + values[values.len() / 2]) / 2;
            println!("summary phase={phase} metric={name} samples={} min_us={:.3} median_us={:.3} p95_us={:.3} p99_us={:.3} max_us={:.3}",
                values.len(), micros(values[0]), micros(median), micros(nearest_rank(95)), micros(nearest_rank(99)), micros(values[values.len() - 1]),
            );
        }
    }
    Ok(())
}

fn micros(time: Duration) -> f64 {
    time.as_secs_f64() * 1_000_000.0
}

fn report_shutdown(c: &Counters, state: &str) {
    println!("shutdown state={state} submissions={} map_requests={} maps_observed={} unmaps={} staging_allocations={} staging_reuses={} staging_retirements={} peak_outstanding={}",
        c.submissions, c.map_requests, c.mapped, c.unmapped, c.staging_allocations, c.staging_reuses, c.staging_retirements, c.peak_outstanding,
    );
}
