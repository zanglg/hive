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
use std::cell::RefCell;
use std::{
    collections::HashSet,
    env, fs,
    io::{self, BufRead, Write},
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
        Commands::Sync { repo } => sync_repo_interactive(&paths, &repo),
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
            install_package_impl(&paths, &package, None)?;
            Ok(String::new())
        }
        Commands::List => list_packages(&paths),
        Commands::Sync { repo } => {
            sync_repo_noninteractive(&paths, &repo)?;
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

struct TerminalSyncPrompts<R, W> {
    input: RefCell<R>,
    output: RefCell<W>,
}

pub trait InstallPrompts {
    fn select_binaries(&self, package: &str, candidates: &[String]) -> Result<Vec<String>, String>;
}

struct TerminalInstallPrompts<R, W> {
    input: RefCell<R>,
    output: RefCell<W>,
}

impl<R, W> TerminalSyncPrompts<R, W> {
    fn new(input: R, output: W) -> Self {
        Self {
            input: RefCell::new(input),
            output: RefCell::new(output),
        }
    }
}

impl<R, W> TerminalInstallPrompts<R, W> {
    fn new(input: R, output: W) -> Self {
        Self {
            input: RefCell::new(input),
            output: RefCell::new(output),
        }
    }
}

impl<R: BufRead, W: Write> TerminalSyncPrompts<R, W> {
    fn read_line(&self, prompt_name: &str) -> Result<String, String> {
        let mut line = String::new();
        self.input
            .borrow_mut()
            .read_line(&mut line)
            .map_err(|error| format!("failed to read {prompt_name}: {error}"))?;
        Ok(line.trim().to_string())
    }
}

impl<R: BufRead, W: Write> TerminalInstallPrompts<R, W> {
    fn read_line(&self, prompt_name: &str) -> Result<String, String> {
        let mut line = String::new();
        self.input
            .borrow_mut()
            .read_line(&mut line)
            .map_err(|error| format!("failed to read {prompt_name}: {error}"))?;
        Ok(line.trim().to_string())
    }
}

impl<R: BufRead, W: Write> sync::SyncPrompts for TerminalSyncPrompts<R, W> {
    fn select_asset(
        &self,
        repo: &str,
        release_tag: &str,
        assets: &[String],
    ) -> Result<sync::PromptInput<String>, String> {
        {
            let mut output = self.output.borrow_mut();
            writeln!(
                output,
                "Select asset for current platform from {repo} release {release_tag}:"
            )
            .map_err(|error| format!("failed to write prompt: {error}"))?;
            for (index, asset) in assets.iter().enumerate() {
                writeln!(output, "{}. {}", index + 1, asset)
                    .map_err(|error| format!("failed to write prompt: {error}"))?;
            }
            write!(output, "Selection: ")
                .map_err(|error| format!("failed to write prompt: {error}"))?;
            output
                .flush()
                .map_err(|error| format!("failed to flush prompt: {error}"))?;
        }
        let selection = self.read_line("asset selection")?;
        if selection.is_empty() {
            return Ok(sync::PromptInput::Default);
        }
        Ok(sync::PromptInput::Value(selection))
    }

    fn input_binaries(
        &self,
        package: &str,
        asset_name: &str,
        suggested_binaries: &[String],
    ) -> Result<sync::PromptInput<Vec<String>>, String> {
        {
            let mut output = self.output.borrow_mut();
            writeln!(
                output,
                "Enter binaries for package `{package}` from `{asset_name}` (comma-separated)."
            )
            .map_err(|error| format!("failed to write prompt: {error}"))?;
            writeln!(output, "Suggested: {}", suggested_binaries.join(", "))
                .map_err(|error| format!("failed to write prompt: {error}"))?;
            write!(output, "Binaries: ")
                .map_err(|error| format!("failed to write prompt: {error}"))?;
            output
                .flush()
                .map_err(|error| format!("failed to flush prompt: {error}"))?;
        }
        let input = self.read_line("binary list")?;
        if input.is_empty() {
            return Ok(sync::PromptInput::Default);
        }
        let binaries = input
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if binaries.is_empty() {
            return Err("binary list cannot be empty".to_string());
        }
        Ok(sync::PromptInput::Value(binaries))
    }
}

impl<R: BufRead, W: Write> InstallPrompts for TerminalInstallPrompts<R, W> {
    fn select_binaries(&self, package: &str, candidates: &[String]) -> Result<Vec<String>, String> {
        {
            let mut output = self.output.borrow_mut();
            writeln!(
                output,
                "Select binaries for package `{package}` (comma-separated numbers):"
            )
            .map_err(|error| format!("failed to write prompt: {error}"))?;
            for (index, candidate) in candidates.iter().enumerate() {
                writeln!(output, "{}. {}", index + 1, candidate)
                    .map_err(|error| format!("failed to write prompt: {error}"))?;
            }
            write!(output, "Selection: ")
                .map_err(|error| format!("failed to write prompt: {error}"))?;
            output
                .flush()
                .map_err(|error| format!("failed to flush prompt: {error}"))?;
        }

        let selection = self.read_line("binary selection")?;
        if selection.is_empty() {
            return Err("binary selection cannot be empty".to_string());
        }

        let mut selected = Vec::new();
        for token in selection.split(',') {
            let index = token.trim();
            if index.is_empty() {
                return Err("binary selection cannot be empty".to_string());
            }

            let candidate = index
                .parse::<usize>()
                .ok()
                .and_then(|value| value.checked_sub(1))
                .and_then(|value| candidates.get(value))
                .cloned();

            match candidate {
                Some(candidate) => {
                    if !selected.contains(&candidate) {
                        selected.push(candidate);
                    }
                }
                None => return Err(format!("selected binary `{index}` was not found")),
            }
        }

        if selected.is_empty() {
            return Err("binary selection cannot be empty".to_string());
        }

        Ok(selected)
    }
}

fn sync_repo_interactive(paths: &HivePaths, repo: &str) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let prompts = TerminalSyncPrompts::new(stdin.lock(), stdout.lock());
    match github_api_base_override() {
        Some(api_base) => {
            sync::sync_repo_with_api_base_and_prompt(paths, repo, &api_base, &prompts)
        }
        None => sync::sync_repo_with_prompt(paths, repo, &prompts),
    }
}

fn sync_repo_noninteractive(paths: &HivePaths, repo: &str) -> Result<(), String> {
    match github_api_base_override() {
        Some(api_base) => sync::sync_repo_with_api_base(paths, repo, &api_base),
        None => sync::sync_repo(paths, repo),
    }
}

fn github_api_base_override() -> Option<String> {
    env::var("HIVE_GITHUB_API_BASE")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn default_paths() -> Result<HivePaths, String> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME is not set".to_string())?;
    Ok(HivePaths::from_home(home))
}

fn install_package(paths: &HivePaths, package: &str) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let prompts = TerminalInstallPrompts::new(stdin.lock(), stdout.lock());
    install_package_impl(paths, package, Some(&prompts))
}

pub fn install_package_with_prompts(
    paths: &HivePaths,
    package: &str,
    prompts: &dyn InstallPrompts,
) -> Result<(), String> {
    install_package_impl(paths, package, Some(prompts))
}

fn install_package_impl(
    paths: &HivePaths,
    package: &str,
    prompts: Option<&dyn InstallPrompts>,
) -> Result<(), String> {
    let repo = ManifestRepository::new(paths.manifest_dirs.clone());
    let (manifest_path, mut manifest) = repo.load(package)?;
    let platform = Platform::current()?;
    let current_platform = platform.to_string();
    let mut artifact = manifest.artifact_for(platform)?.clone();
    let uses_missing_binaries_fallback = artifact.binaries.is_empty();
    let original_manifest_contents = if uses_missing_binaries_fallback {
        Some(
            fs::read_to_string(&manifest_path)
                .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?,
        )
    } else {
        None
    };
    if uses_missing_binaries_fallback && prompts.is_none() {
        return Err(format!(
            "manifest is missing binaries for the current platform `{current_platform}`"
        ));
    }
    let archive_kind = ArchiveKind::parse(&artifact.archive)?;
    let http = proxy::build_http_client()?;

    let download_path = download_to_cache(&http, paths, &artifact.url, package, &manifest.version)?;
    let installer = Installer::new(paths.package_store.clone());
    let version_install_dir = paths.package_store.join(package).join(&manifest.version);
    let install_backup = if uses_missing_binaries_fallback {
        backup_existing_install_dir(&version_install_dir)?
    } else {
        None
    };
    let install_dir = match installer.install_archive(
        &manifest.name,
        &manifest.version,
        &download_path,
        &artifact.checksum,
        archive_kind,
        &artifact.binaries,
    ) {
        Ok(install_dir) => install_dir,
        Err(error) => {
            if uses_missing_binaries_fallback {
                restore_install_backup(&version_install_dir, install_backup.as_deref())?;
            }
            return Err(error);
        }
    };
    let mut manifest_persisted = false;

    if uses_missing_binaries_fallback {
        let selected_binaries = resolve_selected_binaries(
            package,
            prompts,
            &install_dir,
            &current_platform,
        )
        .and_then(|selected_binaries| {
            persist_selected_binaries(
                &manifest_path,
                &mut manifest,
                &current_platform,
                &selected_binaries,
            )?;
            manifest_persisted = true;
            Ok(selected_binaries)
        });

        match selected_binaries {
            Ok(selected_binaries) => artifact.binaries = selected_binaries,
            Err(error) => {
                rollback_missing_binaries_early_failure(
                    &install_dir,
                    install_backup.as_deref(),
                    &manifest_path,
                    original_manifest_contents.as_deref(),
                    manifest_persisted,
                )?;
                return Err(error);
            }
        }
    }

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
            if uses_missing_binaries_fallback {
                rollback_missing_binaries_early_failure(
                    &install_dir,
                    install_backup.as_deref(),
                    &manifest_path,
                    original_manifest_contents.as_deref(),
                    manifest_persisted,
                )?;
            } else if install_dir.exists() {
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
            match export_targets_through_current(&current_dir, &exported) {
                Ok(value) => value,
                Err(error) => {
                    rollback_missing_binaries_late_failure(
                        &install_dir,
                        install_backup.as_deref(),
                        &manifest_path,
                        original_manifest_contents.as_deref(),
                        manifest_persisted,
                    )?;
                    return Err(error);
                }
            };
        if let Err(error) = activate_version(&paths.shim_dir, &active_targets) {
            rollback_missing_binaries_late_failure(
                &install_dir,
                install_backup.as_deref(),
                &manifest_path,
                original_manifest_contents.as_deref(),
                manifest_persisted,
            )?;
            return Err(error);
        }
        state.active = Some(manifest.version.clone());
        if let Err(error) = store.save_package(&state) {
            let _ = rollback_activation_after_state_failure(
                &paths.shim_dir,
                &current_dir,
                &exported,
                &desired_names,
            );
            rollback_missing_binaries_late_failure(
                &install_dir,
                install_backup.as_deref(),
                &manifest_path,
                original_manifest_contents.as_deref(),
                manifest_persisted,
            )?;
            return Err(error);
        }
        if let Err(error) = set_package_current(&current_dir, &install_dir) {
            let _ = store.save_package(&previous_state);
            if previous_current.is_none() {
                let _ = remove_shims_by_names(&paths.shim_dir, &desired_names);
            }
            rollback_missing_binaries_late_failure(
                &install_dir,
                install_backup.as_deref(),
                &manifest_path,
                original_manifest_contents.as_deref(),
                manifest_persisted,
            )?;
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
            rollback_missing_binaries_late_failure(
                &install_dir,
                install_backup.as_deref(),
                &manifest_path,
                original_manifest_contents.as_deref(),
                manifest_persisted,
            )?;
            return Err(error);
        }
        discard_install_backup(install_backup.as_deref())?;
        return Ok(());
    }

    if let Err(error) = store.save_package(&state) {
        rollback_missing_binaries_late_failure(
            &install_dir,
            install_backup.as_deref(),
            &manifest_path,
            original_manifest_contents.as_deref(),
            manifest_persisted,
        )?;
        return Err(error);
    }
    discard_install_backup(install_backup.as_deref())?;
    Ok(())
}

fn resolve_selected_binaries(
    package: &str,
    prompts: Option<&dyn InstallPrompts>,
    install_dir: &Path,
    current_platform: &str,
) -> Result<Vec<String>, String> {
    let prompts = prompts.ok_or_else(|| {
        format!("manifest is missing binaries for the current platform `{current_platform}`")
    })?;
    let candidates = crate::installer::list_executable_candidates(install_dir)?;
    if candidates.is_empty() {
        return Err(format!(
            "manifest is missing binaries for the current platform `{current_platform}`, and no executable candidates were found"
        ));
    }

    let selected = prompts.select_binaries(package, &candidates)?;
    if selected.is_empty() {
        return Err("binary selection cannot be empty".to_string());
    }

    Ok(selected)
}

fn persist_selected_binaries(
    manifest_path: &Path,
    manifest: &mut crate::manifest::Manifest,
    current_platform: &str,
    binaries: &[String],
) -> Result<(), String> {
    manifest.set_binaries_for_platform(current_platform, binaries.to_vec())?;

    if let Some(github) = manifest
        .source
        .as_mut()
        .and_then(|source| source.github.as_mut())
    {
        if let Some(selection) = github.platform.get_mut(current_platform) {
            selection.binaries = binaries.to_vec();
        }
    }

    atomic_write_file(
        manifest_path,
        &manifest.to_toml()?,
        "failed to write",
    )
}

fn backup_existing_install_dir(install_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !install_dir.exists() {
        return Ok(None);
    }

    let backup_name = format!(
        "{}.install-backup",
        install_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("invalid install path `{}`", install_dir.display()))?
    );
    let backup_path = install_dir.with_file_name(backup_name);
    if backup_path.exists() {
        return Err(format!(
            "pre-existing install backup is blocking install: {}",
            backup_path.display()
        ));
    }
    fs::rename(install_dir, &backup_path)
        .map_err(|error| format!("failed to move {} aside: {error}", install_dir.display()))?;
    Ok(Some(backup_path))
}

fn restore_install_backup(install_dir: &Path, install_backup: Option<&Path>) -> Result<(), String> {
    let Some(install_backup) = install_backup else {
        return Ok(());
    };

    if install_dir.exists() {
        fs::remove_dir_all(install_dir)
            .map_err(|error| format!("failed to clean {}: {error}", install_dir.display()))?;
    }
    fs::rename(install_backup, install_dir)
        .map_err(|error| format!("failed to restore {}: {error}", install_dir.display()))
}

fn discard_install_backup(install_backup: Option<&Path>) -> Result<(), String> {
    let Some(install_backup) = install_backup else {
        return Ok(());
    };

    if install_backup.exists() {
        fs::remove_dir_all(install_backup)
            .map_err(|error| format!("failed to remove {}: {error}", install_backup.display()))?;
    }
    Ok(())
}

fn restore_manifest_contents(manifest_path: &Path, original_manifest: Option<&str>) -> Result<(), String> {
    let Some(original_manifest) = original_manifest else {
        return Ok(());
    };

    atomic_write_file(manifest_path, original_manifest, "failed to restore")
}

fn atomic_write_file(path: &Path, contents: &str, error_verb: &str) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid file path `{}`", path.display()))?;
    let parent_dir = path
        .parent()
        .ok_or_else(|| format!("invalid file path `{}`", path.display()))?;
    let temp_path = path.with_file_name(format!(
        ".{file_name}.tmp.{}",
        std::process::id()
    ));

    let write_result = (|| -> Result<(), String> {
        let mut file = fs::File::create(&temp_path)
            .map_err(|error| format!("{error_verb} {}: {error}", path.display()))?;
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("{error_verb} {}: {error}", path.display()))?;
        file.sync_all()
            .map_err(|error| format!("{error_verb} {}: {error}", path.display()))?;
        fs::rename(&temp_path, path)
            .map_err(|error| format!("{error_verb} {}: {error}", path.display()))?;
        fs::File::open(parent_dir)
            .and_then(|dir| dir.sync_all())
            .map_err(|error| format!("{error_verb} {}: {error}", path.display()))
    })();

    if write_result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    write_result
}

fn rollback_missing_binaries_early_failure(
    install_dir: &Path,
    install_backup: Option<&Path>,
    manifest_path: &Path,
    original_manifest: Option<&str>,
    manifest_persisted: bool,
) -> Result<(), String> {
    let mut rollback_errors = Vec::new();

    if install_dir.exists() {
        if let Err(error) = fs::remove_dir_all(install_dir)
            .map_err(|error| format!("failed to clean {}: {error}", install_dir.display()))
        {
            rollback_errors.push(error);
        }
    }
    if manifest_persisted {
        if let Err(error) = restore_manifest_contents(manifest_path, original_manifest) {
            rollback_errors.push(error);
        }
    }
    if let Err(error) = restore_install_backup(install_dir, install_backup) {
        rollback_errors.push(error);
    }

    if rollback_errors.is_empty() {
        Ok(())
    } else {
        Err(rollback_errors.join("; "))
    }
}

fn rollback_missing_binaries_late_failure(
    install_dir: &Path,
    install_backup: Option<&Path>,
    manifest_path: &Path,
    original_manifest: Option<&str>,
    manifest_persisted: bool,
) -> Result<(), String> {
    let mut rollback_errors = Vec::new();

    if manifest_persisted {
        if let Err(error) = restore_manifest_contents(manifest_path, original_manifest) {
            rollback_errors.push(error);
        }
    }
    let install_result = if install_backup.is_some() {
        restore_install_backup(install_dir, install_backup)
    } else if install_dir.exists() {
        fs::remove_dir_all(install_dir)
            .map_err(|error| format!("failed to clean {}: {error}", install_dir.display()))
    } else {
        Ok(())
    };

    if let Err(error) = install_result {
        rollback_errors.push(error);
    }

    if rollback_errors.is_empty() {
        Ok(())
    } else {
        Err(rollback_errors.join("; "))
    }
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
