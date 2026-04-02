use crate::{
    activation::activate_version,
    cli::{Cli, Commands},
    config::HivePaths,
    installer::{ArchiveKind, Installer},
    manifest::ManifestRepository,
    platform::Platform,
    proxy,
    state::{InstalledPackage, StateStore},
    sync,
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

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
        Commands::Sync { repo } => sync_repo(&paths, &repo),
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
        Commands::Sync { repo } => {
            sync_repo(&paths, &repo)?;
            Ok(String::new())
        }
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

fn sync_repo(_paths: &HivePaths, _repo: &str) -> Result<(), String> {
    sync::sync_repo(_paths, _repo)
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
    let http = proxy::build_http_client()?;

    let download_path = download_to_cache(&http, paths, &artifact.url, package, &manifest.version)?;
    let installer = Installer::new(paths.package_store.clone());
    let install_dir = installer.install_archive(
        &manifest.name,
        &manifest.version,
        &download_path,
        &artifact.checksum,
        archive_kind,
        &artifact.binaries,
    )?;

    let exported = match artifact
        .binaries
        .iter()
        .map(|binary| {
            if !crate::installer::path_exists_within_tree(&install_dir, binary)? {
                return Err(format!(
                    "declared binary missing after extraction: {}",
                    install_dir.join(binary).display()
                ));
            }
            Ok((binary.clone(), install_dir.join(binary)))
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
    let previous_state = state.clone();
    let package_root = paths.package_store.join(package);
    let current_dir = package_root.join("current");
    let previous_current = fs::read_link(&current_dir).ok();

    if !state
        .versions
        .iter()
        .any(|version| version == &manifest.version)
    {
        state.versions.push(manifest.version.clone());
        state.versions.sort();
    }

    if state.active.is_none() {
        let (active_targets, desired_names) =
            export_targets_through_current(&current_dir, &exported)?;
        activate_version(&paths.shim_dir, &active_targets)?;
        state.active = Some(manifest.version.clone());
        if let Err(error) = store.save_package(&state) {
            let _ = rollback_activation_after_state_failure(
                &paths.shim_dir,
                &current_dir,
                &exported,
                &desired_names,
            );
            return Err(error);
        }
        if let Err(error) = set_package_current(&current_dir, &install_dir) {
            let _ = store.save_package(&previous_state);
            if previous_current.is_none() {
                let _ = remove_shims_by_names(&paths.shim_dir, &desired_names);
            }
            return Err(error);
        }
        if let Err(error) =
            remove_stale_package_shims(&paths.shim_dir, &package_root, &desired_names)
        {
            let _ = restore_package_current(&current_dir, previous_current.as_deref());
            if previous_current.is_none() {
                let _ = remove_shims_by_names(&paths.shim_dir, &desired_names);
            }
            let _ = store.save_package(&previous_state);
            return Err(error);
        }
        return Ok(());
    }

    store.save_package(&state)
}

fn use_package(paths: &HivePaths, package: &str, version: &str) -> Result<(), String> {
    let repo = ManifestRepository::new(paths.manifest_dirs.clone());
    let (_, manifest) = repo.load(package)?;
    let artifact = manifest.artifact_for(Platform::current()?)?;
    let install_dir = paths.package_store.join(package).join(version);
    let package_root = paths.package_store.join(package);
    let current_dir = package_root.join("current");
    let previous_current = fs::read_link(&current_dir).ok();
    let exported = artifact
        .binaries
        .iter()
        .map(|binary| {
            if !crate::installer::path_exists_within_tree(&install_dir, binary)? {
                return Err(format!(
                    "installed binary not found for package `{package}` version `{version}`: {}",
                    install_dir.join(binary).display()
                ));
            }
            Ok((binary.clone(), install_dir.join(binary)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (exported, desired_names) = export_targets_through_current(&current_dir, &exported)?;

    activate_version(&paths.shim_dir, &exported)?;
    let store = StateStore::new(paths.state_dir.clone());
    let previous_state = store
        .load_package(package)?
        .ok_or_else(|| format!("package `{package}` is not installed"))?;
    if let Err(error) = store.update_active_version(package, version) {
        let _ = rollback_activation_after_state_failure(
            &paths.shim_dir,
            &current_dir,
            &exported,
            &desired_names,
        );
        return Err(error);
    }
    if let Err(error) = set_package_current(&current_dir, &install_dir) {
        let _ = store.save_package(&previous_state);
        if previous_current.is_none() {
            let _ = remove_shims_by_names(&paths.shim_dir, &desired_names);
        }
        return Err(error);
    }
    if let Err(error) = remove_stale_package_shims(&paths.shim_dir, &package_root, &desired_names) {
        let _ = restore_package_current(&current_dir, previous_current.as_deref());
        if previous_current.is_none() {
            let _ = remove_shims_by_names(&paths.shim_dir, &desired_names);
        }
        let _ = store.save_package(&previous_state);
        return Err(error);
    }
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
        remove_shims_for_install_dir(&paths.shim_dir, &install_dir)?;
        remove_package_current(&install_dir)?;
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
    let repo = ManifestRepository::new(paths.manifest_dirs.clone());
    let (_, manifest) = repo.load(package)?;
    let artifact = manifest.artifact_for(Platform::current()?)?;

    match artifact.binaries.as_slice() {
        [] => Err(format!("manifest for `{package}` declares no binaries")),
        [binary] => {
            let shim_name = Path::new(binary)
                .file_name()
                .ok_or_else(|| format!("invalid binary path `{binary}`"))?
                .to_string_lossy()
                .to_string();
            let path = fs::read_link(paths.shim_dir.join(shim_name))
                .map_err(|error| format!("failed to resolve shim for `{package}`: {error}"))?;
            Ok(path.display().to_string())
        }
        _ => Err(format!(
            "which `{package}` is ambiguous for packages with multiple binaries"
        )),
    }
}

fn remove_shims_for_install_dir(shim_dir: &Path, install_dir: &Path) -> Result<(), String> {
    if !shim_dir.exists() {
        return Ok(());
    }

    let current_dir = install_dir
        .parent()
        .map(|parent| parent.join("current"))
        .unwrap_or_else(|| install_dir.join("current"));

    for entry in fs::read_dir(shim_dir)
        .map_err(|error| format!("failed to read {}: {error}", shim_dir.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let target = match fs::read_link(&path) {
            Ok(target) => target,
            Err(_) => continue,
        };

        if target.starts_with(install_dir) || target.starts_with(&current_dir) {
            fs::remove_file(&path)
                .map_err(|error| format!("failed to remove {}: {error}", path.display()))?;
        }
    }

    Ok(())
}

fn export_targets_through_current(
    current_dir: &Path,
    binaries: &[(String, PathBuf)],
) -> Result<(Vec<(String, PathBuf)>, HashSet<String>), String> {
    let mut targets = Vec::with_capacity(binaries.len());
    let mut names = HashSet::with_capacity(binaries.len());

    for (binary, _) in binaries {
        let shim_name = Path::new(binary)
            .file_name()
            .ok_or_else(|| format!("invalid binary path `{binary}`"))?
            .to_string_lossy()
            .to_string();
        if !names.insert(shim_name.clone()) {
            return Err(format!("duplicate shim name `{shim_name}`"));
        }
        targets.push((shim_name, current_dir.join(binary)));
    }

    Ok((targets, names))
}

fn remove_stale_package_shims(
    shim_dir: &Path,
    package_root: &Path,
    desired_names: &HashSet<String>,
) -> Result<(), String> {
    if !shim_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(shim_dir)
        .map_err(|error| format!("failed to read {}: {error}", shim_dir.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let name = match path.file_name().and_then(|value| value.to_str()) {
            Some(value) => value,
            None => continue,
        };
        if desired_names.contains(name) {
            continue;
        }

        let target = match fs::read_link(&path) {
            Ok(target) => target,
            Err(_) => continue,
        };

        if target.starts_with(package_root) {
            fs::remove_file(&path)
                .map_err(|error| format!("failed to remove {}: {error}", path.display()))?;
        }
    }

    Ok(())
}

fn restore_package_current(
    current_dir: &Path,
    previous_current: Option<&Path>,
) -> Result<(), String> {
    if let Some(previous_current) = previous_current {
        set_package_current(current_dir, previous_current)
    } else if current_dir.symlink_metadata().is_ok() {
        fs::remove_file(current_dir)
            .map_err(|error| format!("failed to remove {}: {error}", current_dir.display()))
    } else {
        Ok(())
    }
}

fn remove_shims_by_names(shim_dir: &Path, desired_names: &HashSet<String>) -> Result<(), String> {
    if !shim_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(shim_dir)
        .map_err(|error| format!("failed to read {}: {error}", shim_dir.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let name = match path.file_name().and_then(|value| value.to_str()) {
            Some(value) => value,
            None => continue,
        };
        if desired_names.contains(name) {
            fs::remove_file(&path)
                .map_err(|error| format!("failed to remove {}: {error}", path.display()))?;
        }
    }

    Ok(())
}

fn rollback_activation_after_state_failure(
    shim_dir: &Path,
    current_dir: &Path,
    exported: &[(String, PathBuf)],
    desired_names: &HashSet<String>,
) -> Result<(), String> {
    if current_dir.symlink_metadata().is_ok() {
        activate_version(shim_dir, exported)
    } else {
        remove_shims_by_names(shim_dir, desired_names)
    }
}

fn remove_package_current(install_dir: &Path) -> Result<(), String> {
    let current_dir = install_dir
        .parent()
        .map(|parent| parent.join("current"))
        .unwrap_or_else(|| install_dir.join("current"));

    if let Ok(metadata) = current_dir.symlink_metadata() {
        let remove_result = if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&current_dir)
        } else {
            fs::remove_file(&current_dir)
        };
        remove_result
            .map_err(|error| format!("failed to remove {}: {error}", current_dir.display()))?;
    }

    Ok(())
}

fn set_package_current(current_dir: &Path, install_dir: &Path) -> Result<(), String> {
    if let Ok(metadata) = current_dir.symlink_metadata() {
        let remove_result = if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(current_dir)
        } else {
            fs::remove_file(current_dir)
        };
        remove_result
            .map_err(|error| format!("failed to replace {}: {error}", current_dir.display()))?;
    }

    if let Some(parent) = current_dir.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    std::os::unix::fs::symlink(install_dir, current_dir)
        .map_err(|error| format!("failed to link {}: {error}", current_dir.display()))
}

fn download_to_cache(
    http: &reqwest::blocking::Client,
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

    let response = http.get(url).send().map_err(|error| error.to_string())?;
    let response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    fs::write(&cache_path, &bytes)
        .map_err(|error| format!("failed to write {}: {error}", cache_path.display()))?;
    Ok(cache_path)
}
