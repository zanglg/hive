use crate::{
    config::HivePaths,
    github::{GitHubClient, Release},
    manifest::{Artifact, GitHubSource, Manifest, ManifestSource},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";

pub fn sync_repo(paths: &HivePaths, repo: &str) -> Result<(), String> {
    sync_repo_with_api_base(paths, repo, DEFAULT_GITHUB_API_BASE)
}

pub fn sync_repo_with_api_base(paths: &HivePaths, repo: &str, api_base: &str) -> Result<(), String> {
    let (_, package) = parse_repo(repo)?;
    let manifest_path = paths.manifest_dirs[0].join(format!("{package}.toml"));
    let existing = if manifest_path.exists() {
        Some(
            Manifest::from_toml(
                &fs::read_to_string(&manifest_path)
                    .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?,
            )
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
        });

    if source.repo != repo {
        return Err(format!("stored GitHub repo does not match `{repo}`"));
    }

    let client = GitHubClient::new(api_base);
    let release = client.latest_release(repo, &source.channel)?;
    let manifest = build_manifest_from_release(package, &source, existing.as_ref(), &release, &client)?;

    if existing.as_ref() == Some(&manifest) {
        return Ok(());
    }

    fs::create_dir_all(&paths.manifest_dirs[0])
        .map_err(|error| format!("failed to create {}: {error}", paths.manifest_dirs[0].display()))?;
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

        let binaries = existing
            .and_then(|manifest| manifest.platform.get(platform_key))
            .map(|artifact| artifact.binaries.clone())
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
        || existing
            .map(|manifest| {
                manifest
                    .platform
                    .keys()
                    .all(|platform_key| platform.contains_key(platform_key))
            })
            == Some(false)
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
    tag_name
        .strip_prefix('v')
        .unwrap_or(tag_name)
        .to_string()
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
