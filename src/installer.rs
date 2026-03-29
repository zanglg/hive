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
        declared_binaries: &[String],
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

        if let Err(error) = normalize_extracted_layout(&temp_dir, declared_binaries) {
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

fn normalize_extracted_layout(temp_dir: &Path, declared_binaries: &[String]) -> Result<(), String> {
    let mut entries = fs::read_dir(temp_dir)
        .map_err(|error| format!("failed to inspect {}: {error}", temp_dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {}: {error}", temp_dir.display()))?;

    if binaries_exist_under(temp_dir, declared_binaries)? {
        return Ok(());
    }

    if entries.len() != 1 {
        return Ok(());
    }

    let entry = entries.pop().unwrap();
    let entry_file_type = entry
        .file_type()
        .map_err(|error| format!("failed to inspect {}: {error}", temp_dir.display()))?;
    if !entry_file_type.is_dir() {
        return Ok(());
    }

    let entry_path = entry.path();
    if !binaries_exist_under(&entry_path, declared_binaries)? {
        return Ok(());
    }

    for child in fs::read_dir(&entry_path)
        .map_err(|error| format!("failed to inspect {}: {error}", entry_path.display()))?
    {
        let child = child.map_err(|error| format!("failed to inspect {}: {error}", entry_path.display()))?;
        let destination = temp_dir.join(child.file_name());
        fs::rename(child.path(), &destination).map_err(|error| {
            format!(
                "failed to normalize extracted layout from {}: {error}",
                entry_path.display()
            )
        })?;
    }

    fs::remove_dir(&entry_path).map_err(|error| {
        format!(
            "failed to remove wrapper directory {}: {error}",
            entry_path.display()
        )
    })?;

    Ok(())
}

fn binaries_exist_under(dir: &Path, declared_binaries: &[String]) -> Result<bool, String> {
    for binary in declared_binaries {
        if !dir.join(binary).exists() {
            return Ok(false);
        }
    }

    Ok(true)
}
