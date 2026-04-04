use crate::platform::Platform;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ManifestSource>,
    pub platform: BTreeMap<String, Artifact>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct ManifestSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHubSource>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct GitHubSource {
    pub repo: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub platform: BTreeMap<String, GitHubPlatformSelection>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct GitHubPlatformSelection {
    pub asset: String,
    pub binaries: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Artifact {
    pub url: String,
    pub checksum: String,
    pub archive: String,
    pub binaries: Vec<String>,
}

impl Manifest {
    pub fn from_toml(contents: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(contents)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|error| format!("failed to serialize manifest: {error}"))
    }

    pub fn artifact_for(&self, platform: Platform) -> Result<&Artifact, String> {
        self.platform
            .get(&platform.to_string())
            .ok_or_else(|| format!("manifest does not support platform {platform}"))
    }

    pub fn set_binaries_for_platform(
        &mut self,
        platform: &str,
        binaries: Vec<String>,
    ) -> Result<(), String> {
        let artifact = self
            .platform
            .get_mut(platform)
            .ok_or_else(|| format!("manifest does not support platform {platform}"))?;

        artifact.binaries = binaries;

        Ok(())
    }
}

pub struct ManifestRepository {
    roots: Vec<PathBuf>,
}

impl ManifestRepository {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn find(&self, package: &str) -> Result<PathBuf, String> {
        let mut matches = Vec::new();

        for root in &self.roots {
            collect_if_file(&mut matches, &root.join(format!("{package}.toml")));
            collect_if_file(&mut matches, &root.join(package).join("manifest.toml"));
        }

        match matches.len() {
            0 => Err(format!("manifest not found for package `{package}`")),
            1 => Ok(matches.remove(0)),
            _ => Err(format!("ambiguous manifest for package `{package}`")),
        }
    }

    pub fn load(&self, package: &str) -> Result<(PathBuf, Manifest), String> {
        let path = self.find(package)?;
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let manifest = Manifest::from_toml(&contents)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;

        if manifest.name != package {
            return Err(format!(
                "manifest name `{}` does not match requested package `{package}`",
                manifest.name
            ));
        }

        Ok((path, manifest))
    }
}

fn collect_if_file(matches: &mut Vec<PathBuf>, path: &Path) {
    if path.is_file() {
        matches.push(path.to_path_buf());
    }
}
