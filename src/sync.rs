use crate::{
    config::HivePaths,
    github::{GitHubClient, Release},
    manifest::{Artifact, GitHubPlatformSelection, GitHubSource, Manifest, ManifestSource},
    platform::Platform,
    proxy,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";

pub enum PromptInput<T> {
    Value(T),
    Default,
}

pub trait SyncPrompts {
    fn select_asset(
        &self,
        repo: &str,
        release_tag: &str,
        assets: &[String],
    ) -> Result<PromptInput<String>, String>;

    fn input_binaries(
        &self,
        package: &str,
        asset_name: &str,
        suggested_binaries: &[String],
    ) -> Result<PromptInput<Vec<String>>, String>;
}

pub fn sync_repo(paths: &HivePaths, repo: &str) -> Result<(), String> {
    sync_repo_with_api_base(paths, repo, DEFAULT_GITHUB_API_BASE)
}

pub fn sync_repo_with_prompt(
    paths: &HivePaths,
    repo: &str,
    prompts: &dyn SyncPrompts,
) -> Result<(), String> {
    sync_repo_with_api_base_and_prompt(paths, repo, DEFAULT_GITHUB_API_BASE, prompts)
}

pub fn sync_repo_with_api_base(
    paths: &HivePaths,
    repo: &str,
    api_base: &str,
) -> Result<(), String> {
    sync_repo_with_api_base_impl(paths, repo, api_base, None)
}

pub fn sync_repo_with_api_base_and_prompt(
    paths: &HivePaths,
    repo: &str,
    api_base: &str,
    prompts: &dyn SyncPrompts,
) -> Result<(), String> {
    sync_repo_with_api_base_impl(paths, repo, api_base, Some(prompts))
}

fn sync_repo_with_api_base_impl(
    paths: &HivePaths,
    repo: &str,
    api_base: &str,
    prompts: Option<&dyn SyncPrompts>,
) -> Result<(), String> {
    let (_, package) = parse_repo(repo)?;
    let manifest_path = paths.manifest_dirs[0].join(format!("{package}.toml"));
    let existing =
        if manifest_path.exists() {
            Some(
                Manifest::from_toml(&fs::read_to_string(&manifest_path).map_err(|error| {
                    format!("failed to read {}: {error}", manifest_path.display())
                })?)
                .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?,
            )
        } else {
            None
        };

    let source = existing
        .as_ref()
        .and_then(|manifest| manifest.source.as_ref())
        .and_then(|source| source.github.as_ref())
        .cloned()
        .unwrap_or(GitHubSource {
            repo: repo.to_string(),
            channel: "stable".to_string(),
            platform: BTreeMap::new(),
        });

    if source.repo != repo {
        return Err(format!("stored GitHub repo does not match `{repo}`"));
    }

    let client = GitHubClient::new(api_base, proxy::build_http_client()?);
    let release = client.latest_release(repo, &source.channel)?;
    let current_platform = Platform::current()?;
    let current_platform_key = current_platform.to_string();
    let source = resolve_current_platform_selection(
        package,
        &source,
        existing.as_ref(),
        &release,
        current_platform,
        &current_platform_key,
        prompts,
    )?;
    let manifest = build_manifest_from_release(
        package,
        &source,
        existing.as_ref(),
        &release,
        &client,
        &current_platform_key,
    )?;

    if existing.as_ref() == Some(&manifest) {
        return Ok(());
    }

    fs::create_dir_all(&paths.manifest_dirs[0]).map_err(|error| {
        format!(
            "failed to create {}: {error}",
            paths.manifest_dirs[0].display()
        )
    })?;
    fs::write(&manifest_path, manifest.to_toml()?)
        .map_err(|error| format!("failed to write {}: {error}", manifest_path.display()))
}

fn build_manifest_from_release(
    package: &str,
    source: &GitHubSource,
    existing: Option<&Manifest>,
    release: &Release,
    client: &GitHubClient,
    current_platform: &str,
) -> Result<Manifest, String> {
    let selection = source.platform.get(current_platform).ok_or_else(|| {
        format!("missing GitHub asset selection for current platform `{current_platform}`")
    })?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == selection.asset)
        .ok_or_else(|| {
            format!(
                "selected asset `{}` was not found in release `{}`",
                selection.asset, release.tag_name
            )
        })?;
    let archive = archive_kind_from_name(&asset.name).ok_or_else(|| {
        format!(
            "selected asset `{}` has unsupported archive format",
            asset.name
        )
    })?;
    let bytes = read_artifact_bytes(client, &asset.browser_download_url)?;
    let checksum = format!("sha256:{:x}", Sha256::digest(&bytes));
    let mut platform = existing
        .map(|manifest| manifest.platform.clone())
        .unwrap_or_default();
    platform.insert(
        current_platform.to_string(),
        Artifact {
            url: asset.browser_download_url.clone(),
            checksum,
            archive: archive.to_string(),
            binaries: selection.binaries.clone(),
        },
    );

    Ok(Manifest {
        name: package.to_string(),
        version: normalize_version(&release.tag_name),
        source: Some(ManifestSource {
            github: Some(source.clone()),
        }),
        platform,
    })
}

fn resolve_current_platform_selection(
    package: &str,
    source: &GitHubSource,
    existing: Option<&Manifest>,
    release: &Release,
    current_platform: Platform,
    current_platform_key: &str,
    prompts: Option<&dyn SyncPrompts>,
) -> Result<GitHubSource, String> {
    let saved_selection = source.platform.get(current_platform_key).cloned();
    let default_asset = saved_selection
        .as_ref()
        .map(|selection| selection.asset.clone())
        .or_else(|| {
            infer_asset_name_from_existing_artifact(existing, current_platform_key, release)
        });
    let default_binaries = saved_selection
        .as_ref()
        .map(|selection| selection.binaries.clone())
        .or_else(|| {
            existing
                .and_then(|manifest| manifest.platform.get(current_platform_key))
                .map(|artifact| artifact.binaries.clone())
        });
    let suggested_binaries = default_binaries
        .clone()
        .unwrap_or_else(|| vec![package.to_string()]);
    let selected_asset = match prompts {
        Some(prompts) => {
            let asset_names = installable_release_assets(release)?;
            let selected_asset = resolve_prompted_asset_name(
                current_platform_key,
                &asset_names,
                prompts.select_asset(&source.repo, &release.tag_name, &asset_names)?,
                default_asset.clone(),
            )?;
            ensure_selected_asset_matches_current_platform(
                &selected_asset,
                current_platform,
                current_platform_key,
            )?;
            selected_asset
        }
        None => default_asset.ok_or_else(|| {
            format!("missing GitHub asset selection for current platform `{current_platform_key}`")
        })?,
    };
    ensure_supported_asset_name(&selected_asset)?;
    let binaries = match prompts {
        Some(prompts) => resolve_prompted_binaries(
            current_platform_key,
            prompts.input_binaries(package, &selected_asset, &suggested_binaries)?,
            default_binaries,
        )?,
        None => suggested_binaries,
    };
    let mut prompted = source.clone();
    prompted.platform.insert(
        current_platform_key.to_string(),
        GitHubPlatformSelection {
            asset: selected_asset,
            binaries,
        },
    );
    Ok(prompted)
}

fn resolve_prompted_asset_name(
    current_platform: &str,
    assets: &[String],
    selection: PromptInput<String>,
    default_asset: Option<String>,
) -> Result<String, String> {
    match selection {
        PromptInput::Value(selection) => resolve_selected_asset_name(assets, &selection),
        PromptInput::Default => default_asset.ok_or_else(|| {
            format!(
                "asset selection cannot be empty without a saved default for current platform `{current_platform}`"
            )
        }),
    }
}

fn resolve_prompted_binaries(
    current_platform: &str,
    binaries: PromptInput<Vec<String>>,
    default_binaries: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    match binaries {
        PromptInput::Value(binaries) => Ok(binaries),
        PromptInput::Default => default_binaries.ok_or_else(|| {
            format!(
                "binary list cannot be empty without saved binaries for current platform `{current_platform}`"
            )
        }),
    }
}

fn resolve_selected_asset_name(assets: &[String], selection: &str) -> Result<String, String> {
    if let Ok(index) = selection.parse::<usize>() {
        if (1..=assets.len()).contains(&index) {
            let asset = &assets[index - 1];
            return Ok(asset.clone());
        }
    }

    assets
        .iter()
        .find(|asset| asset.as_str() == selection)
        .cloned()
        .ok_or_else(|| format!("selected asset `{selection}` was not found"))
}

fn parse_repo(repo: &str) -> Result<(&str, &str), String> {
    let mut parts = repo.split('/');
    let owner = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("invalid GitHub repo `{repo}`"))?;
    let package = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("invalid GitHub repo `{repo}`"))?;
    if parts.next().is_some() {
        return Err(format!("invalid GitHub repo `{repo}`"));
    }
    Ok((owner, package))
}

fn normalize_version(tag_name: &str) -> String {
    tag_name.strip_prefix('v').unwrap_or(tag_name).to_string()
}

fn infer_asset_name_from_existing_artifact(
    existing: Option<&Manifest>,
    current_platform: &str,
    release: &Release,
) -> Option<String> {
    let manifest = existing?;
    let asset_name = manifest
        .platform
        .get(current_platform)
        .and_then(|artifact| asset_name_from_url(&artifact.url))?;
    let current_version = &manifest.version;
    let release_version = normalize_version(&release.tag_name);
    let old_tag = format!("v{current_version}");
    let mut candidates = Vec::new();

    push_candidate(&mut candidates, asset_name.to_string());
    if current_version != &release_version {
        push_candidate(
            &mut candidates,
            asset_name.replace(current_version, &release_version),
        );
    }
    if old_tag != release.tag_name {
        push_candidate(
            &mut candidates,
            asset_name.replace(&old_tag, &release.tag_name),
        );
    }

    candidates
        .into_iter()
        .find(|candidate| release.assets.iter().any(|asset| asset.name == *candidate))
}

fn asset_name_from_url(url: &str) -> Option<&str> {
    let url = url.split(['?', '#']).next()?;
    url.rsplit('/').next().filter(|value| !value.is_empty())
}

fn push_candidate(candidates: &mut Vec<String>, candidate: String) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn ensure_supported_asset_name(name: &str) -> Result<(), String> {
    archive_kind_from_name(name)
        .map(|_| ())
        .ok_or_else(|| format!("selected asset `{name}` has unsupported archive format"))
}

fn installable_release_assets(release: &Release) -> Result<Vec<String>, String> {
    let assets = release
        .assets
        .iter()
        .filter(|asset| archive_kind_from_name(&asset.name).is_some())
        .map(|asset| asset.name.clone())
        .collect::<Vec<_>>();
    if assets.is_empty() {
        return Err(format!(
            "release `{}` has no installable assets",
            release.tag_name
        ));
    }
    Ok(assets)
}

fn ensure_selected_asset_matches_current_platform(
    selected_asset: &str,
    current_platform: Platform,
    current_platform_key: &str,
) -> Result<(), String> {
    if let Some(detected_platform) = detect_supported_platform_from_asset_name(selected_asset) {
        if detected_platform != current_platform {
            return Err(format!(
                "selected asset `{selected_asset}` does not match current platform `{current_platform_key}`"
            ));
        }
    }
    Ok(())
}

fn detect_supported_platform_from_asset_name(name: &str) -> Option<Platform> {
    let tokens = normalized_asset_tokens(name);
    let matches = [
        Platform::LinuxX86_64,
        Platform::LinuxAarch64,
        Platform::MacosX86_64,
        Platform::MacosAarch64,
    ]
    .into_iter()
    .filter(|platform| asset_name_matches_platform(&tokens, *platform))
    .collect::<Vec<_>>();

    match matches.as_slice() {
        [platform] => Some(*platform),
        _ => None,
    }
}

fn asset_name_matches_platform(tokens: &BTreeSet<String>, platform: Platform) -> bool {
    let (os_tokens, arch_tokens) = platform_detection_tokens(platform);
    os_tokens.iter().any(|token| tokens.contains(*token))
        && arch_tokens.iter().any(|token| tokens.contains(*token))
}

fn normalized_asset_tokens(name: &str) -> BTreeSet<String> {
    let tokens = name
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut normalized = BTreeSet::new();

    for token in &tokens {
        normalized.insert(token.clone());
    }

    for pair in tokens.windows(2) {
        normalized.insert(format!("{}{}", pair[0], pair[1]));
    }

    normalized
}

fn platform_detection_tokens(
    platform: Platform,
) -> (&'static [&'static str], &'static [&'static str]) {
    match platform {
        Platform::LinuxX86_64 => (&["linux"], &["x8664", "amd64", "x64"]),
        Platform::LinuxAarch64 => (&["linux"], &["aarch64", "arm64"]),
        Platform::MacosX86_64 => (
            &["macos", "darwin", "apple", "osx"],
            &["x8664", "amd64", "x64"],
        ),
        Platform::MacosAarch64 => (&["macos", "darwin", "apple", "osx"], &["aarch64", "arm64"]),
    }
}

fn archive_kind_from_name(name: &str) -> Option<&'static str> {
    if name.ends_with(".tar.gz") {
        Some("tar.gz")
    } else if name.ends_with(".tar.xz") {
        Some("tar.xz")
    } else if name.ends_with(".zip") {
        Some("zip")
    } else {
        None
    }
}

fn read_artifact_bytes(client: &GitHubClient, url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(PathBuf::from(path))
            .map_err(|error| format!("failed to read artifact {url}: {error}"));
    }

    client.download_bytes(url)
}
