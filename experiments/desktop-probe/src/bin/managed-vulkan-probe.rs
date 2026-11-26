//! Explicit experimental entry; never selected by ordinary cargo run.
use desktop_probe::managed_vulkan::NativeVulkanLaunch;
fn main() {
    let arguments: Result<Vec<_>, _> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string())
        .collect();
    let launch = arguments
        .map_err(|_| "native arguments must be UTF-8".to_owned())
        .and_then(NativeVulkanLaunch::parse);
    let launch = match launch {
        Ok(launch) => launch,
        Err(error) => {
            eprintln!("[NATIVE ARGUMENTS] {error}");
            std::process::exit(2);
        }
    };
    match desktop_probe::run_managed_vulkan(launch) {
        Ok(summary) => eprintln!("[EXPERIMENTAL NATIVE] {summary:?}"),
        Err(error) => {
            eprintln!("[EXPERIMENTAL NATIVE] {error}\n{:?}", error.0);
            std::process::exit(1);
        }
    }
}
