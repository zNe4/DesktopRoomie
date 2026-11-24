use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AdapterPolicy {
    #[default]
    Default,
    Software,
    Hardware,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Cpu,
    Integrated,
    Discrete,
    Other,
}
impl AdapterPolicy {
    pub fn accepts(self, class: DeviceClass) -> bool {
        match self {
            Self::Default => true,
            Self::Software => class == DeviceClass::Cpu,
            Self::Hardware => matches!(class, DeviceClass::Integrated | DeviceClass::Discrete),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LaunchConfig {
    pub adapter: AdapterPolicy,
    driver: Option<PathBuf>,
    report: Option<PathBuf>,
}
impl LaunchConfig {
    /// Literal argv only. One adapter, one driver manifest, one optional report.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        if args.len() > 6 || !args.len().is_multiple_of(2) {
            return Err("invalid launch arguments".into());
        }
        let mut result = Self::default();
        let mut adapter_seen = false;
        for pair in args.as_chunks::<2>().0 {
            match pair[0].as_str() {
                "--adapter" if !adapter_seen => {
                    result.adapter = match pair[1].as_str() {
                        "default" => AdapterPolicy::Default,
                        "software" => AdapterPolicy::Software,
                        "hardware" => AdapterPolicy::Hardware,
                        _ => return Err("invalid adapter policy".into()),
                    };
                    adapter_seen = true;
                }
                "--driver" if result.driver.is_none() => {
                    result.driver = Some(validate_driver(Path::new(&pair[1]))?)
                }
                "--acceptance-report" if result.report.is_none() => {
                    result.report = Some(PathBuf::from(&pair[1]));
                }
                _ => return Err("unknown or duplicate launch option".into()),
            }
        }
        Ok(result)
    }
    pub fn driver(&self) -> Option<&Path> {
        self.driver.as_deref()
    }
    pub fn report(&self) -> Option<&Path> {
        self.report.as_deref()
    }
    /// Call once in the single-threaded worker entry point, before wgpu exists.
    /// The accepted supervisor clears the environment before exec. No new
    /// supervisor environment passthrough is needed or introduced.
    pub fn configure_driver(&self) {
        std::env::remove_var("VK_DRIVER_FILES");
        if let Some(path) = &self.driver {
            std::env::set_var("VK_DRIVER_FILES", path);
        }
    }
}
fn validate_driver(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("driver path must be UTF-8")?;
    if text.len() > 4096 || text.contains([':', '\0']) || !path.is_absolute() {
        return Err("driver must be one absolute manifest path".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "driver manifest unavailable")?;
    let text = canonical.to_str().ok_or("driver path must be UTF-8")?;
    if text.len() > 4096
        || text.contains([':', '\0'])
        || canonical.extension().is_none_or(|x| x != "json")
        || !canonical.is_file()
    {
        return Err("driver must be one regular JSON manifest".into());
    }
    std::fs::File::open(&canonical).map_err(|_| "driver manifest unreadable")?;
    Ok(canonical)
}
