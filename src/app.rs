use crate::{
    activation::activate_version,
    cli::{Cli, Commands},
    config::HivePaths,
    installer::{ArchiveKind, Installer},
    manifest::ManifestRepository,
    platform::Platform,
    state::{InstalledPackage, StateStore},
};
use std::{fs, path::PathBuf};

pub fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::List | Commands::Which { .. } => {
            let output = run_capture(cli, default_paths()?)?;
            if !output.is_empty() {
                println!("{output}");
            }
            Ok(())
        }
        _ => run_with_paths(cli, default_paths()?),
    }
}

pub fn run_with_paths(cli: Cli, paths: HivePaths) -> Result<(), String> {
    match cli.command {
        Commands::Install { package } => install_package(&paths, &package),
        Commands::List => {
            let output = list_packages(&paths)?;
            if !output.is_empty() {
                println!("{output}");
            }
            Ok(())
        }
        Commands::Use { package, version } => use_package(&paths, &package, &version),
        Commands::Uninstall {
            package,
            version,
            force,
        } => uninstall_package(&paths, &package, &version, force),
        Commands::Which { package } => {
            let output = which_package(&paths, &package)?;
            println!("{output}");
            Ok(())
        }
    }
}

pub fn run_capture(cli: Cli, paths: HivePaths) -> Result<String, String> {
    match cli.command {
        Commands::Install { package } => {
            install_package(&paths, &package)?;
            Ok(String::new())
        }
        Commands::List => list_packages(&paths),
        Commands::Use { package, version } => {
            use_package(&paths, &package, &version)?;
            Ok(String::new())
        }
        Commands::Uninstall {
            package,
            version,
            force,
        } => {
            uninstall_package(&paths, &package, &version, force)?;
            Ok(String::new())
        }
        Commands::Which { package } => which_package(&paths, &package),
    }
}

fn default_paths() -> Result<HivePaths, String> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME is not set".to_string())?;
    Ok(HivePaths::from_home(home))
}

fn install_package(paths: &HivePaths, package: &str) -> Result<(), String> {
    let repo = ManifestRepository::new(paths.manifest_dirs.clone());
    let (_, manifest) = repo.load(package)?;
    let platform = Platform::current()?;
    let artifact = manifest.artifact_for(platform)?.clone();
    let archive_kind = ArchiveKind::parse(&artifact.archive)?;

    let download_path = download_to_cache(paths, &artifact.url, package, &manifest.version)?;
    let installer = Installer::new(paths.package_store.clone());
    let install_dir = installer.install_archive(
        &manifest.name,
        &manifest.version,
        &download_path,
        &artifact.checksum,
        archive_kind,
    )?;

    let exported = match artifact
        .binaries
        .iter()
        .map(|binary| {
            let target = install_dir.join(binary);
            if !target.exists() {
                return Err(format!(
                    "declared binary missing after extraction: {}",
                    target.display()
                ));
            }
            Ok((binary.clone(), target))
        })
        .collect::<Result<Vec<_>, String>>()
    {
        Ok(value) => value,
        Err(error) => {
            if install_dir.exists() {
                fs::remove_dir_all(&install_dir).map_err(|remove_error| {
                    format!("failed to clean {}: {remove_error}", install_dir.display())
                })?;
            }
            return Err(error);
        }
    };

    let store = StateStore::new(paths.state_dir.clone());
    let mut state = store
        .load_package(&manifest.name)?
        .unwrap_or(InstalledPackage {
            name: manifest.name.clone(),
            versions: Vec::new(),
            active: None,
        });

    if !state
        .versions
        .iter()
        .any(|version| version == &manifest.version)
    {
        state.versions.push(manifest.version.clone());
        state.versions.sort();
    }

    if state.active.is_none() {
        activate_version(&paths.shim_dir, &exported)?;
        state.active = Some(manifest.version.clone());
    }

    store.save_package(&state)
}

fn use_package(paths: &HivePaths, package: &str, version: &str) -> Result<(), String> {
    let repo = ManifestRepository::new(paths.manifest_dirs.clone());
    let (_, manifest) = repo.load(package)?;
    let artifact = manifest.artifact_for(Platform::current()?)?;
    let install_dir = paths.package_store.join(package).join(version);
    let exported = artifact
        .binaries
        .iter()
        .map(|binary| {
            let target = install_dir.join(binary);
            if !target.exists() {
                return Err(format!(
                    "installed binary not found for package `{package}` version `{version}`: {}",
                    target.display()
                ));
            }
            Ok((binary.clone(), target))
        })
        .collect::<Result<Vec<_>, String>>()?;

    activate_version(&paths.shim_dir, &exported)?;
    let store = StateStore::new(paths.state_dir.clone());
    store.update_active_version(package, version)?;
    Ok(())
}

fn uninstall_package(
    paths: &HivePaths,
    package: &str,
    version: &str,
    force: bool,
) -> Result<(), String> {
    let store = StateStore::new(paths.state_dir.clone());
    let updated = store.remove_version(package, version, force)?;

    let install_dir = paths.package_store.join(package).join(version);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .map_err(|error| format!("failed to remove {}: {error}", install_dir.display()))?;
    }

    if updated.active.is_none() {
        let link = paths.shim_dir.join(package);
        if link.symlink_metadata().is_ok() {
            fs::remove_file(&link)
                .map_err(|error| format!("failed to remove {}: {error}", link.display()))?;
        }
    }

    Ok(())
}

fn list_packages(paths: &HivePaths) -> Result<String, String> {
    let store = StateStore::new(paths.state_dir.clone());
    let mut lines = Vec::new();
    for package in store.list_packages()? {
        for version in &package.versions {
            let marker = if package.active.as_deref() == Some(version.as_str()) {
                " *"
            } else {
                ""
            };
            lines.push(format!("{} {}{}", package.name, version, marker));
        }
    }
    Ok(lines.join("\n"))
}

fn which_package(paths: &HivePaths, package: &str) -> Result<String, String> {
    let path = fs::read_link(paths.shim_dir.join(package))
        .map_err(|error| format!("failed to resolve shim for `{package}`: {error}"))?;
    Ok(path.display().to_string())
}

fn download_to_cache(
    paths: &HivePaths,
    url: &str,
    package: &str,
    version: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(&paths.state_dir)
        .map_err(|error| format!("failed to create {}: {error}", paths.state_dir.display()))?;
    let cache_path = paths.state_dir.join(format!("{package}-{version}.archive"));

    if let Some(path) = url.strip_prefix("file://") {
        fs::copy(path, &cache_path)
            .map_err(|error| format!("failed to copy fixture archive into cache: {error}"))?;
        return Ok(cache_path);
    }

    let response = reqwest::blocking::get(url).map_err(|error| error.to_string())?;
    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    fs::write(&cache_path, &bytes)
        .map_err(|error| format!("failed to write {}: {error}", cache_path.display()))?;
    Ok(cache_path)
}
