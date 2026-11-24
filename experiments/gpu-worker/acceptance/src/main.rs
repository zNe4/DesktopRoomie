use roomie_worker_acceptance::{
    adapter::{Adapter, Event},
    oracle,
    policy::Workload,
    telemetry::{Report, RunDirectory},
    workload,
};
use roomie_worker_supervisor::Readiness;
use std::{path::PathBuf, time::Duration};

const HELP:&str="Headless acceptance (native execution requires separate owner authorization).\n\
Usage: roomie-worker-acceptance --run --worker ABSOLUTE_PATH --adapter software|hardware --workload exact|stress|cancel|cancel-before|unsupported|init-failure [--driver ABSOLUTE_JSON] [--telemetry]\n\
GPU-free fixture: replace --adapter POLICY with --synthetic. No automatic fallback.\n\
No arguments: help only. Telemetry disabled by default. Native workloads NOT executed by tests.\n";
fn run(args: &[String]) -> Result<(), String> {
    let mut worker = None;
    let mut adapter = None;
    let mut synthetic = false;
    let mut mode = "exact";
    let mut driver = None;
    let mut telemetry = false;
    let mut run = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--run" if !run => run = true,
            "--synthetic" if !synthetic => synthetic = true,
            "--telemetry" if !telemetry => telemetry = true,
            flag @ ("--worker" | "--adapter" | "--driver" | "--workload") => {
                index += 1;
                let value = args.get(index).ok_or("option requires a value")?;
                match flag {
                    "--worker" if worker.is_none() => worker = Some(PathBuf::from(value)),
                    "--adapter" if adapter.is_none() => adapter = Some(value.as_str()),
                    "--driver" if driver.is_none() => driver = Some(value.as_str()),
                    "--workload" => mode = value,
                    _ => return Err("duplicate option".into()),
                }
            }
            _ => return Err("invalid option".into()),
        }
        index += 1;
    }
    let worker = worker.ok_or("worker executable required")?;
    if !run
        || !worker.is_absolute()
        || (synthetic && (adapter.is_some() || driver.is_some()))
        || (!synthetic && !adapter.is_some_and(|a| ["software", "hardware"].contains(&a)))
    {
        return Err("explicit run, absolute worker and one backend policy required".into());
    }
    let workload = Workload::parse(mode)?;
    let policy = workload.policy();
    if mode == "init-failure" && !telemetry {
        return Err(
            "initialization-failure proof requires --telemetry; missing metrics are unknown".into(),
        );
    }
    if mode == "init-failure" && !synthetic && driver.is_none() {
        return Err("unavailable-ICD workload requires an explicit --driver manifest".into());
    }
    let directory = if telemetry {
        Some(RunDirectory::create()?)
    } else {
        None
    };
    let report_path = directory.as_ref().map(RunDirectory::report_path);
    let mut child_args: Vec<String> = if synthetic {
        vec![if mode == "init-failure" {
            "init-rejection"
        } else {
            "good"
        }
        .into()]
    } else {
        vec!["--adapter".into(), adapter.unwrap().into()]
    };
    if let Some(driver) = driver {
        child_args.extend(["--driver".into(), driver.into()]);
    }
    if let Some(path) = &report_path {
        println!("telemetry_path={}", path.display());
        child_args.extend([
            "--acceptance-report".into(),
            path.to_str().ok_or("report path UTF-8")?.into(),
        ]);
    }
    let refs: Vec<_> = child_args.iter().map(String::as_str).collect();
    println!(
        "freshness_policy workload={mode} budget_ms={} authorities=D1_and_supervisor policy={policy:?} purpose={}",
        policy.freshness.as_millis(),
        if workload == Workload::Exact {
            "bounded_exact_parity_verification_allowance"
        } else {
            "accepted_default"
        }
    );
    let mut owner = Adapter::spawn(&worker, &refs, 1, policy)?;
    let mut stress_evidence = None;
    let result = match mode {
        "exact" => workload::exact_observed(&mut owner, |r| {
            println!("exact warmup={} measured={} phases={:?} warmup_IPC_quantiles_50_95_99={:?} warmup_end_to_end_quantiles_50_95_99={:?} measured_IPC_quantiles_50_95_99={:?} measured_end_to_end_quantiles_50_95_99={:?}",
                r.warmup,r.measured,r.phases,quantiles(&r.warmup_ipc),quantiles(&r.warmup_end_to_end),quantiles(&r.ipc),quantiles(&r.end_to_end));
            println!("{}", r.latency_reference_summary());
        }).map(|_| ()),
        "stress" => workload::stress(&mut owner).and_then(|r| {
            println!("stress target_hz=60 window_valid_hz={} window_settled_frame_hz={} window=[2s,12s) window_denominator={:?} actual_measurement_elapsed={:?} pacing_elapsed={:?} window_valid={} window_obsolete={} warmup={:?} measured={:?} cadence_quantiles_50_95_99={:?} lateness_quantiles_50_95_99={:?} IPC_quantiles_50_95_99={:?} end_to_end_quantiles_50_95_99={:?} failure={:?}",
                r.throughput(),r.settled_frame_throughput(),r.window_elapsed,r.measured_elapsed,r.pacing_elapsed,r.window_valid,r.window_obsolete,r.warmup,r.measured,quantiles(&r.cadence),quantiles(&r.lateness),quantiles(&r.ipc),quantiles(&r.end_to_end),r.failure);
            let result = r.failure.clone().map_or(Ok(()), Err);
            stress_evidence = Some(r);
            result
        }),
        "init-failure" => {
            let initialized = owner.wait_ready();
            let outcome = owner.finish()?;
            println!("initialization diagnostic={:?} containment={outcome:?}", owner.diagnostic());
            if initialized.is_ok() { Err("expected initialization rejection".into()) }
            else { workload::initialization_rejection(&owner, !synthetic) }
        }
        _ => cancellation(&mut owner,mode),
    };
    println!(
        "domain={} counters={:?} outcome={:?} exit_code={:?} unresolved_D1={:?}",
        if synthetic {
            "synthetic (zero native GPU work)"
        } else {
            "native Vulkan; owner-run evidence only"
        },
        owner.counters(),
        owner.outcome(),
        owner.exit_code(),
        owner.outstanding()
    );
    println!(
        "owner_initialization_duration={:?} owner_shutdown_duration={:?}",
        owner.initialization_duration(),
        owner.shutdown_duration()
    );
    result?;
    if let Some(path) = &report_path {
        let report = Report::read(path)?;
        if mode == "exact" {
            report.exact125(!synthetic)?;
        }
        if ["cancel", "cancel-before", "unsupported"].contains(&mode) {
            report.completed(
                u128::from(owner.counters().valid + owner.counters().obsolete),
                !synthetic,
            )?;
        }
        if !synthetic && mode != "init-failure" {
            report.adapter(adapter.unwrap())?;
        }
        if let Some(stress) = &stress_evidence {
            report.stress(stress, !synthetic)?;
        }
        if mode == "init-failure" {
            report.initialization_failure(!synthetic, owner.outcome(), owner.exit_code())?;
        }
        println!(
            "telemetry=complete domain={} timing_scope=whole_worker_lifetime_including_warmup_and_late_settlement timing_denominator_timed_operations={:?} worker_encode_ns={:?} worker_wait_ns={:?}",
            report.kind()?,
            report.number("timed_operations"),
            report.number("encode_ns"),
            report.number("wait_ns")
        );
    } else {
        println!("telemetry=disabled resource/timing counters UNVERIFIED");
    }
    println!(
        "correctness workload completed; independent review/native acceptance remain separate"
    );
    Ok(())
}
fn quantiles(samples: &roomie_worker_acceptance::scheduler::Samples) -> [Option<Duration>; 3] {
    [
        samples.percentile(50),
        samples.percentile(95),
        samples.percentile(99),
    ]
}
fn cancellation(owner: &mut Adapter, mode: &str) -> Result<(), String> {
    let result = (|| {
        owner.wait_ready()?;
        let mut request = oracle::request(1, Duration::ZERO);
        if mode == "unsupported" {
            use desktop_probe::presentation::{Action, ActionIntent, Facing, FrameRequest};
            request = FrameRequest::new(
                request.identity(),
                ActionIntent {
                    action: Action::Walk,
                    facing: Facing::BackRight,
                },
                request.extent(),
                Duration::ZERO,
                Duration::ZERO,
            )
            .map_err(|e| format!("{e:?}"))?;
        }
        let id = owner.submit(request, owner.now())?;
        if mode == "cancel-before" {
            owner.shutdown(owner.now());
        } else if mode != "unsupported" {
            owner.progress(
                Readiness {
                    writable: true,
                    ..Default::default()
                },
                owner.now(),
            )?;
            if owner.outcome().job.is_none_or(|j| {
                j.transmission != roomie_worker_supervisor::machine::Transmission::Complete
            }) {
                return Err("dispatch barrier not established".into());
            }
            owner.cancel();
        }
        match owner.wait_event()? {
            Event::Obsolete(job) if mode == "cancel" && job == id => Ok(()),
            Event::Withdrawn(job) if mode == "cancel-before" && job == id => Ok(()),
            Event::Rejected(job) if mode == "unsupported" && job == id => Ok(()),
            e => Err(format!("unexpected cancellation result: {e:?}")),
        }
    })();
    let outcome = owner.finish()?;
    result.and_then(|_| workload::controlled(outcome))
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).take(18).collect();
    if args.is_empty() || args == ["--help"] {
        print!("{HELP}");
        return;
    }
    if args.len() == 18 {
        eprintln!("too many arguments");
        std::process::exit(2);
    }
    if let Err(e) = run(&args) {
        eprintln!("FAIL: {e}");
        std::process::exit(1);
    }
}
