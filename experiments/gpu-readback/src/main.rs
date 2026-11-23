#[cfg(feature = "native-vulkan")]
mod gpu;
#[cfg(any(feature = "native-vulkan", test))]
mod layout;
#[cfg(any(feature = "native-vulkan", test))]
mod oracle;
#[cfg(any(feature = "native-vulkan", test))]
mod readback;
#[cfg(feature = "native-vulkan")]
mod run;

#[cfg(any(feature = "native-vulkan", test))]
use std::time::Duration;

#[cfg(any(feature = "native-vulkan", test))]
type HostError = Box<dyn std::error::Error>;

#[cfg(any(feature = "native-vulkan", test))]
const WIDTH: usize = 160;
#[cfg(any(feature = "native-vulkan", test))]
const HEIGHT: usize = 160;
#[cfg(any(feature = "native-vulkan", test))]
const PHASES: usize = 5;
#[cfg(any(feature = "native-vulkan", test))]
const PERIOD: Duration = Duration::from_millis(300);

const USAGE: &str = "Usage: cargo run --manifest-path experiments/gpu-readback/Cargo.toml --release --locked --features native-vulkan -- --run";

fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    let mut args = std::env::args_os().skip(1);
    match (
        args.next().as_deref().and_then(std::ffi::OsStr::to_str),
        args.next(),
    ) {
        (None | Some("--help"), None) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        (Some("--run"), None) => {
            #[cfg(feature = "native-vulkan")]
            {
                run::experiment()
            }
            #[cfg(not(feature = "native-vulkan"))]
            {
                eprintln!("Native Vulkan requires the native-vulkan feature.\n{USAGE}");
                ExitCode::from(2)
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
