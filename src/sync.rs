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
    let source = match prompts {
        Some(prompts) if existing.is_none() => {
            prompt_for_initial_platform_selection(package, &source, &release, prompts)?
        }
        None => source,
        Some(_) => source,
    };
    let manifest =
        build_manifest_from_release(package, &source, existing.as_ref(), &release, &client)?;

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
) -> Result<Manifest, String> {
    let mut platform = BTreeMap::new();

    for asset in &release.assets {
        let Some(platform_key) = map_asset_to_platform(&asset.name) else {
            continue;
        };
        let Some(archive) = archive_kind_from_name(&asset.name) else {
            continue;
        };

        let binaries = source
            .platform
            .get(platform_key)
            .filter(|selection| selection.asset == asset.name)
            .map(|selection| selection.binaries.clone())
            .or_else(|| {
                existing
                    .and_then(|manifest| manifest.platform.get(platform_key))
                    .map(|artifact| artifact.binaries.clone())
            })
            .unwrap_or_else(|| vec![package.to_string()]);
        let bytes = read_artifact_bytes(client, &asset.browser_download_url)?;
        let checksum = format!("sha256:{:x}", Sha256::digest(&bytes));

        if platform.contains_key(platform_key) {
            return Err(format!(
                "could not map GitHub assets from release `{}` into supported Hive platforms",
                release.tag_name
            ));
        }

        platform.insert(
            platform_key.to_string(),
            Artifact {
                url: asset.browser_download_url.clone(),
                checksum,
                archive: archive.to_string(),
                binaries,
            },
        );
    }

    if platform.is_empty()
        || existing.map(|manifest| {
            manifest
                .platform
                .keys()
                .all(|platform_key| platform.contains_key(platform_key))
        }) == Some(false)
    {
        return Err(format!(
            "could not map GitHub assets from release `{}` into supported Hive platforms",
            release.tag_name
        ));
    }

    Ok(Manifest {
        name: package.to_string(),
        version: normalize_version(&release.tag_name),
        source: Some(ManifestSource {
            github: Some(source.clone()),
        }),
        platform,
    })
}

fn prompt_for_initial_platform_selection(
    package: &str,
    source: &GitHubSource,
    release: &Release,
    prompts: &dyn SyncPrompts,
) -> Result<GitHubSource, String> {
    let current_platform = Platform::current()?.to_string();
    if source.platform.contains_key(&current_platform) {
        return Ok(source.clone());
    }

    let asset_names = release
        .assets
        .iter()
        .map(|asset| asset.name.clone())
        .collect::<Vec<_>>();
    let selected_asset = resolve_selected_asset_name(
        &asset_names,
        &prompts.select_asset(&source.repo, &release.tag_name, &asset_names)?,
    )?;
    let selected_platform = map_asset_to_platform(&selected_asset)
        .ok_or_else(|| format!("selected asset `{selected_asset}` is not supported"))?;

    if selected_platform != current_platform {
        return Err(format!(
            "selected asset `{selected_asset}` does not match current platform `{current_platform}`"
        ));
    }

    let suggested_binaries = vec![package.to_string()];
    let binaries = prompts.input_binaries(package, &selected_asset, &suggested_binaries)?;
    let mut prompted = source.clone();
    prompted.platform.insert(
        current_platform,
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

fn map_asset_to_platform(name: &str) -> Option<&'static str> {
    match () {
        _ if name.contains("x86_64-unknown-linux") => Some("linux-x86_64"),
        _ if name.contains("aarch64-unknown-linux") => Some("linux-aarch64"),
        _ if name.contains("x86_64-apple-darwin") => Some("macos-x86_64"),
        _ if name.contains("aarch64-apple-darwin") || name.contains("arm64-apple-darwin") => {
            Some("macos-aarch64")
        }
        _ => None,
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
