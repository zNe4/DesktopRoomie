//! Offline verification of one private managed Vulkan evidence directory.
fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(directory) = args.next().filter(|_| args.next().is_none()) else {
        eprintln!("usage: managed-vulkan-verify REPORT_DIRECTORY");
        std::process::exit(2);
    };
    match desktop_probe::managed_vulkan::verify_evidence(directory.as_ref()) {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("correctness evidence failed: {error}");
            std::process::exit(1);
        }
    }
}
