use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub versions: Vec<String>,
    pub active: Option<String>,
}

pub struct StateStore {
    root: PathBuf,
}

impl StateStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn load_package(&self, package: &str) -> Result<Option<InstalledPackage>, String> {
        let path = self.root.join(format!("{package}.json"));
        if !path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        serde_json::from_str(&contents)
            .map(Some)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))
    }

    pub fn save_package(&self, package: &InstalledPackage) -> Result<(), String> {
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("failed to create {}: {error}", self.root.display()))?;
        let path = self.root.join(format!("{}.json", package.name));
        let contents = serde_json::to_string_pretty(package).map_err(|error| error.to_string())?;
        fs::write(&path, contents)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))
    }

    pub fn update_active_version(
        &self,
        package: &str,
        version: &str,
    ) -> Result<InstalledPackage, String> {
        let mut entry = self
            .load_package(package)?
            .ok_or_else(|| format!("package `{package}` is not installed"))?;
        if !entry.versions.iter().any(|installed| installed == version) {
            return Err(format!(
                "package `{package}` does not have version `{version}` installed"
            ));
        }
        entry.active = Some(version.to_string());
        self.save_package(&entry)?;
        Ok(entry)
    }

    pub fn remove_version(
        &self,
        package: &str,
        version: &str,
        force: bool,
    ) -> Result<InstalledPackage, String> {
        let mut entry = self
            .load_package(package)?
            .ok_or_else(|| format!("package `{package}` is not installed"))?;

        if entry.active.as_deref() == Some(version) && !force {
            return Err(format!(
                "cannot uninstall active version `{version}` of package `{package}`"
            ));
        }

        let original_len = entry.versions.len();
        entry.versions.retain(|installed| installed != version);
        if entry.versions.len() == original_len {
            return Err(format!(
                "package `{package}` does not have version `{version}` installed"
            ));
        }

        if entry.active.as_deref() == Some(version) {
            entry.active = None;
        }

        self.save_package(&entry)?;
        Ok(entry)
    }

    pub fn list_packages(&self) -> Result<Vec<InstalledPackage>, String> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut packages = Vec::new();
        for entry in fs::read_dir(&self.root)
            .map_err(|error| format!("failed to read {}: {error}", self.root.display()))?
        {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let contents = fs::read_to_string(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let package = serde_json::from_str(&contents)
                .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
            packages.push(package);
        }
        packages.sort_by(|left: &InstalledPackage, right: &InstalledPackage| {
            left.name.cmp(&right.name)
        });
        Ok(packages)
    }
}
