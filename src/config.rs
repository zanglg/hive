use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HivePaths {
    pub manifest_dirs: Vec<PathBuf>,
    pub package_store: PathBuf,
    pub state_dir: PathBuf,
    pub shim_dir: PathBuf,
}

impl HivePaths {
    pub fn from_home(home: PathBuf) -> Self {
        Self {
            manifest_dirs: vec![home.join(".config/hive/manifests")],
            package_store: home.join(".local/share/hive/pkgs"),
            state_dir: home.join(".local/share/hive/state"),
            shim_dir: home.join(".local/bin/hive"),
        }
    }
}
