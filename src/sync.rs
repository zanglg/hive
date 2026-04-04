use crate::{
    config::HivePaths,
    github::{GitHubClient, Release},
    manifest::{Artifact, GitHubPlatformSelection, GitHubSource, Manifest, ManifestSource},
    platform::Platform,
    proxy,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";

pub trait SyncPrompts {
    fn select_asset(
        &self,
        repo: &str,
        release_tag: &str,
        assets: &[String],
    ) -> Result<String, String>;

    fn input_binaries(
        &self,
        package: &str,
        asset_name: &str,
        suggested_binaries: &[String],
    ) -> Result<Vec<String>, String>;
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
    let current_platform = Platform::current()?.to_string();
    let source = resolve_current_platform_selection(
        package,
        &source,
        existing.as_ref(),
        &release,
        &current_platform,
        prompts,
    )?;
    let manifest = build_manifest_from_release(
        package,
        &source,
        existing.as_ref(),
        &release,
        &client,
        &current_platform,
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
    current_platform: &str,
    prompts: Option<&dyn SyncPrompts>,
) -> Result<GitHubSource, String> {
    let saved_selection = source.platform.get(current_platform).cloned();
    let inferred_asset =
        infer_asset_name_from_existing_artifact(existing, current_platform, release);
    let suggested_binaries = saved_selection
        .as_ref()
        .map(|selection| selection.binaries.clone())
        .or_else(|| {
            existing
                .and_then(|manifest| manifest.platform.get(current_platform))
                .map(|artifact| artifact.binaries.clone())
        })
        .unwrap_or_else(|| vec![package.to_string()]);
    let selected_asset = match prompts {
        Some(prompts) => {
            let asset_names = release
                .assets
                .iter()
                .map(|asset| asset.name.clone())
                .collect::<Vec<_>>();
            resolve_selected_asset_name(
                &asset_names,
                &prompts.select_asset(&source.repo, &release.tag_name, &asset_names)?,
            )?
        }
        None => saved_selection
            .as_ref()
            .map(|selection| selection.asset.clone())
            .or(inferred_asset)
            .ok_or_else(|| {
                format!("missing GitHub asset selection for current platform `{current_platform}`")
            })?,
    };
    ensure_supported_asset_name(&selected_asset)?;
    let binaries = match prompts {
        Some(prompts) => prompts.input_binaries(package, &selected_asset, &suggested_binaries)?,
        None => suggested_binaries,
    };
    let mut prompted = source.clone();
    prompted.platform.insert(
        current_platform.to_string(),
        GitHubPlatformSelection {
            asset: selected_asset,
            binaries,
        },
    );
    Ok(prompted)
}

fn resolve_selected_asset_name(assets: &[String], selection: &str) -> Result<String, String> {
    if let Ok(index) = selection.parse::<usize>() {
        if let Some(asset) = assets.get(index.saturating_sub(1)) {
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
    let asset_name = existing
        .and_then(|manifest| manifest.platform.get(current_platform))
        .and_then(|artifact| asset_name_from_url(&artifact.url))?;

    release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.name.clone())
}

fn asset_name_from_url(url: &str) -> Option<&str> {
    let url = url.split(['?', '#']).next()?;
    url.rsplit('/').next().filter(|value| !value.is_empty())
}

fn ensure_supported_asset_name(name: &str) -> Result<(), String> {
    archive_kind_from_name(name)
        .map(|_| ())
        .ok_or_else(|| format!("selected asset `{name}` has unsupported archive format"))
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
