use crate::control;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CliMode {
    Help,
    Diagnose,
    Owner {
        delay_secs: Option<u64>,
        duration_secs: Option<u64>,
    },
    Control(control::Command),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct UsageError(pub(super) String);

impl UsageError {
    pub(super) fn exit_code(&self) -> control::ExitCode {
        control::ExitCode::Usage
    }
}

// Arguments exclude argv[0]. Parsing has no process, output, or host side effects.
pub(super) fn parse_args<I, S>(args: I) -> Result<CliMode, UsageError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let mut help = false;
    let mut diagnose = false;
    let mut duration_secs = None;
    let mut delay_secs = None;
    let mut command = None;
    while let Some(arg) = args.next() {
        match arg.as_ref() {
            "-h" | "--help" => help = true,
            "--diagnose" => diagnose = true,
            flag @ ("--delay" | "--duration") => {
                let value = args
                    .next()
                    .ok_or_else(|| UsageError(format!("Missing value for {flag} <seconds>")))?;
                let value = value.as_ref().parse::<u64>().map_err(|_| {
                    UsageError(format!("Invalid value for {flag}: '{}'", value.as_ref()))
                })?;
                if flag == "--delay" {
                    delay_secs = Some(value);
                } else {
                    duration_secs = Some(value);
                }
            }
            flag @ ("--hide" | "--show" | "--bring-top" | "--bring-here" | "--recover") => {
                let next = match flag {
                    "--hide" => control::Command::Hide,
                    "--show" => control::Command::Show,
                    "--bring-here" => control::Command::BringHere,
                    "--recover" => control::Command::Recover,
                    _ => control::Command::BringTop,
                };
                if command.replace(next).is_some() {
                    return Err(UsageError("Control commands are mutually exclusive".into()));
                }
            }
            unknown => {
                return Err(UsageError(format!(
                    "Unknown option: '{unknown}'. Use --help for usage."
                )))
            }
        }
    }
    if let Some(command) = command {
        if diagnose || delay_secs.is_some() || duration_secs.is_some() {
            return Err(UsageError(
                "Control commands cannot combine with --diagnose, --delay, or --duration".into(),
            ));
        }
        if !help {
            return Ok(CliMode::Control(command));
        }
    }
    if help {
        Ok(CliMode::Help)
    } else if diagnose {
        Ok(CliMode::Diagnose)
    } else {
        Ok(CliMode::Owner {
            delay_secs,
            duration_secs,
        })
    }
}

pub(super) const HELP: &str = "DesktopRoomie - A00-M03.3: Desktop Probe

Left click toggles color; left drag moves the body.
Right click opens Above/Normal/Below/Dismiss/Quit after release. Outside left click dismisses.

USAGE:
  desktop-probe [OPTIONS]

OPTIONS:
  -h, --help              Show this help message and exit
  --diagnose              Run pure X11 environment diagnostics and exit
  --delay <SECONDS>       Delay in seconds before mapping window (for typing test)
  --duration <SECONDS>    Run for a specified duration in seconds, then exit
  --show       Restore the existing body without activation or workspace/layer changes
  --hide       Minimize the existing body without workspace/layer changes
  --bring-top  Ensure the existing body is not minimized and set Above on its existing workspace
  --bring-here Move the existing body to the caller's current workspace, preserving its effective layer and genuine minimized state; do not activate it or switch the user's workspace
  --recover    Bring the existing body to the caller's frozen current workspace, restore it if minimized, and set Above; confirm placement and input readiness without activation or switching the user's workspace";

pub(super) fn print_help() {
    println!("{HELP}");
}
