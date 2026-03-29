use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

impl ArchiveKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "tar.gz" => Ok(Self::TarGz),
            "zip" => Ok(Self::Zip),
            other => Err(format!("unsupported archive type `{other}`")),
        }
    }
}

pub struct Installer {
    package_store: PathBuf,
}

impl Installer {
    pub fn new(package_store: PathBuf) -> Self {
        Self { package_store }
    }

    pub fn install_archive(
        &self,
        package: &str,
        version: &str,
        archive_path: &Path,
        checksum: &str,
        archive_kind: ArchiveKind,
    ) -> Result<PathBuf, String> {
        let bytes = fs::read(archive_path)
            .map_err(|error| format!("failed to read {}: {error}", archive_path.display()))?;
        let actual = format!("sha256:{:x}", Sha256::digest(&bytes));
        if actual != checksum {
            return Err(format!(
                "checksum mismatch: expected {checksum}, got {actual}"
            ));
        }

        let version_parent = self.package_store.join(package);
        let install_dir = version_parent.join(version);
        let temp_dir = version_parent.join(format!("{version}.tmp"));

        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)
                .map_err(|error| format!("failed to remove {}: {error}", temp_dir.display()))?;
        }
        fs::create_dir_all(&temp_dir)
            .map_err(|error| format!("failed to create {}: {error}", temp_dir.display()))?;

        let extract_result = match archive_kind {
            ArchiveKind::TarGz => {
                let file = fs::File::open(archive_path).map_err(|error| {
                    format!("failed to read {}: {error}", archive_path.display())
                })?;
                let decoder = flate2::read::GzDecoder::new(file);
                let mut archive = tar::Archive::new(decoder);
                archive.unpack(&temp_dir).map_err(|error| error.to_string())
            }
            ArchiveKind::Zip => {
                let file = fs::File::open(archive_path).map_err(|error| {
                    format!("failed to read {}: {error}", archive_path.display())
                })?;
                let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
                archive
                    .extract(&temp_dir)
                    .map_err(|error| error.to_string())
            }
        };

        if let Err(error) = extract_result {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(error);
        }

        if install_dir.exists() {
            fs::remove_dir_all(&install_dir)
                .map_err(|error| format!("failed to remove {}: {error}", install_dir.display()))?;
        }
        fs::create_dir_all(&version_parent)
            .map_err(|error| format!("failed to create {}: {error}", version_parent.display()))?;
        fs::rename(&temp_dir, &install_dir)
            .map_err(|error| format!("failed to move install into place: {error}"))?;
        Ok(install_dir)
    }
}
