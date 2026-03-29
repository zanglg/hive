use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    LinuxX86_64,
    LinuxAarch64,
    MacosX86_64,
    MacosAarch64,
}

impl Platform {
    pub fn current() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("linux", "aarch64") => Ok(Self::LinuxAarch64),
            ("macos", "x86_64") => Ok(Self::MacosX86_64),
            ("macos", "aarch64") => Ok(Self::MacosAarch64),
            (os, arch) => Err(format!("unsupported platform: {os}-{arch}")),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::LinuxX86_64 => "linux-x86_64",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::MacosX86_64 => "macos-x86_64",
            Self::MacosAarch64 => "macos-aarch64",
        };
        f.write_str(value)
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "linux-x86_64" => Ok(Self::LinuxX86_64),
            "linux-aarch64" => Ok(Self::LinuxAarch64),
            "macos-x86_64" => Ok(Self::MacosX86_64),
            "macos-aarch64" => Ok(Self::MacosAarch64),
            _ => Err(format!("unsupported platform key: {value}")),
        }
    }
}
