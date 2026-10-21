use super::*;
use crate::control::{Command, ExitCode};

#[test]
fn default_and_owner_invocations_remain_valid() {
    assert_eq!(
        parse_args(Vec::<String>::new()),
        Ok(CliMode::Owner {
            delay_secs: None,
            duration_secs: None
        })
    );
    assert_eq!(
        parse_args(["--delay", "2", "--duration", "30"]),
        Ok(CliMode::Owner {
            delay_secs: Some(2),
            duration_secs: Some(30)
        })
    );
    assert_eq!(
        parse_args(["--duration", "0", "--delay", "18446744073709551615"]),
        Ok(CliMode::Owner {
            delay_secs: Some(u64::MAX),
            duration_secs: Some(0)
        })
    );
    assert_eq!(
        parse_args(["--delay", "1", "--delay", "2"]),
        Ok(CliMode::Owner {
            delay_secs: Some(2),
            duration_secs: None
        })
    );
    assert_eq!(
        parse_args(["--duration", "5"]),
        Ok(CliMode::Owner {
            delay_secs: None,
            duration_secs: Some(5)
        })
    );
}
#[test]
fn help_and_diagnose_remain_separate_modes() {
    for flag in ["--help", "-h"] {
        assert_eq!(parse_args([flag]), Ok(CliMode::Help));
    }
    assert_eq!(parse_args(["--diagnose"]), Ok(CliMode::Diagnose));
    assert_eq!(
        parse_args(["--delay", "2", "--diagnose", "--duration", "5"]),
        Ok(CliMode::Diagnose)
    );
    assert_eq!(parse_args(["--diagnose", "--help"]), Ok(CliMode::Help));
}
#[test]
fn control_commands_parse_into_independent_caller_role() {
    for (flag, command) in [
        ("--hide", Command::Hide),
        ("--show", Command::Show),
        ("--bring-top", Command::BringTop),
        ("--bring-here", Command::BringHere),
        ("--recover", Command::Recover),
    ] {
        assert_eq!(parse_args([flag]), Ok(CliMode::Control(command)));
    }
}
#[test]
fn help_advertises_both_placement_commands_with_their_semantics() {
    assert!(HELP.contains("A00-M03.3"));
    assert!(HELP.contains("--bring-here"));
    assert!(HELP.contains("genuine minimized state"));
    assert!(HELP.contains("do not activate it or switch the user's workspace"));
    let recover = HELP
        .lines()
        .find(|line| line.contains("--recover"))
        .unwrap();
    for meaning in [
        "existing body",
        "frozen current workspace",
        "restore it if minimized",
        "Above",
        "placement and input readiness",
        "without activation",
        "switching the user's workspace",
    ] {
        assert!(recover.contains(meaning), "{meaning}");
    }
    assert_eq!(
        parse_args(["--recover"]),
        Ok(CliMode::Control(Command::Recover))
    );
    assert_eq!(parse_args(["--recover", "--help"]), Ok(CliMode::Help));
}
#[test]
fn control_conflicts_in_either_order_are_usage_errors() {
    for a in [
        "--hide",
        "--show",
        "--bring-top",
        "--bring-here",
        "--recover",
    ] {
        for b in [
            "--hide",
            "--show",
            "--bring-top",
            "--bring-here",
            "--recover",
            "--diagnose",
        ] {
            for args in [[a, b], [b, a]] {
                assert_eq!(parse_args(args).unwrap_err().exit_code(), ExitCode::Usage);
            }
        }
        for option in ["--delay", "--duration"] {
            for args in [[a, option, "0"], [option, "0", a]] {
                assert_eq!(parse_args(args).unwrap_err().exit_code() as u32, 2);
            }
        }
    }
}
#[test]
fn invalid_missing_overflowing_and_unknown_arguments_are_meaningful() {
    for flag in ["--delay", "--duration"] {
        assert!(parse_args([flag]).unwrap_err().0.contains("Missing value"));
        for value in ["-1", "1.5", "abc", "18446744073709551616", "--diagnose"] {
            let error = parse_args([flag, value]).unwrap_err();
            assert!(error.0.contains(flag));
            assert!(error.0.contains(value));
            assert_eq!(error.exit_code(), ExitCode::Usage);
        }
    }
    let error = parse_args(["--unknown"]).unwrap_err();
    assert!(error.0.contains("Unknown option: '--unknown'"));
    assert_eq!(error.exit_code() as i32, 2);
}
