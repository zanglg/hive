#![allow(dead_code)]

use flate2::{Compression, write::GzEncoder};
use hive::{
    config::HivePaths,
    manifest::{Artifact, GitHubSource, Manifest, ManifestSource},
    state::{InstalledPackage, StateStore},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
    thread,
    time::Duration,
};
use tar::Builder;
use xz2::write::XzEncoder;

pub fn write_tar_gz(archive_path: &Path, source_dir: &Path, file_name: &str) {
    let tar_gz = fs::File::create(archive_path).unwrap();
    let encoder = GzEncoder::new(tar_gz, Compression::default());
    let mut builder = Builder::new(encoder);
    builder
        .append_path_with_name(source_dir.join(file_name), file_name)
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

pub fn write_tar_xz(archive_path: &Path, source_dir: &Path, file_name: &str) {
    let tar_xz = fs::File::create(archive_path).unwrap();
    let encoder = XzEncoder::new(tar_xz, 6);
    let mut builder = Builder::new(encoder);
    builder
        .append_path_with_name(source_dir.join(file_name), file_name)
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

pub fn write_tar_gz_with_wrapper(archive_path: &Path, source_dir: &Path, wrapper_dir: &str) {
    let tar_gz = fs::File::create(archive_path).unwrap();
    let encoder = GzEncoder::new(tar_gz, Compression::default());
    let mut builder = Builder::new(encoder);
    builder.append_dir_all(wrapper_dir, source_dir).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

pub fn write_tar_gz_with_symlink(archive_path: &Path, link_path: &str, link_target: &Path) {
    let tar_gz = fs::File::create(archive_path).unwrap();
    let encoder = GzEncoder::new(tar_gz, Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_link_name(link_target).unwrap();
    header.set_mode(0o777);
    header.set_cksum();
    builder
        .append_data(&mut header, link_path, io::empty())
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

pub fn write_tar_gz_files(archive_path: &Path, files: &[(&Path, &str)]) {
    let tar_gz = fs::File::create(archive_path).unwrap();
    let encoder = GzEncoder::new(tar_gz, Compression::default());
    let mut builder = Builder::new(encoder);
    for (source_path, archive_path_name) in files {
        builder
            .append_path_with_name(source_path, archive_path_name)
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

pub fn fixture_paths(root: &Path) -> HivePaths {
    HivePaths {
        manifest_dirs: vec![root.join("manifests")],
        package_store: root.join("pkgs"),
        state_dir: root.join("state"),
        shim_dir: root.join("bin/hive"),
    }
}

pub fn seed_install_fixture(paths: &HivePaths, package: &str, version: &str) {
    let source_dir = paths.state_dir.join("fixture-source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join(package), "stub-binary").unwrap();

    fs::create_dir_all(&paths.state_dir).unwrap();
    let archive_path = paths.state_dir.join(format!("{package}-{version}.tar.gz"));
    write_tar_gz(&archive_path, &source_dir, package);

    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    write_manifest_with_archive(
        paths,
        package,
        version,
        &archive_path,
        &checksum,
        package,
        "tar.gz",
    );
}

pub fn seed_install_fixture_tar_xz(paths: &HivePaths, package: &str, version: &str) {
    let source_dir = paths.state_dir.join("fixture-source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join(package), "stub-binary").unwrap();

    fs::create_dir_all(&paths.state_dir).unwrap();
    let archive_path = paths.state_dir.join(format!("{package}-{version}.tar.xz"));
    write_tar_xz(&archive_path, &source_dir, package);

    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    write_manifest_with_archive(
        paths,
        package,
        version,
        &archive_path,
        &checksum,
        package,
        "tar.xz",
    );
}

pub fn seed_bad_checksum_fixture(paths: &HivePaths, package: &str, version: &str) {
    seed_install_fixture(paths, package, version);
    let manifest_path = paths.manifest_dirs[0].join(format!("{package}.toml"));
    let contents = fs::read_to_string(&manifest_path).unwrap();
    let bad_checksum = "sha256:deadbeef";
    let updated = contents.replacen("sha256:", bad_checksum, 1);
    fs::write(manifest_path, updated).unwrap();
}

pub fn seed_missing_binary_fixture(paths: &HivePaths, package: &str, version: &str) {
    let source_dir = paths.state_dir.join("fixture-source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("not-rg"), "stub-binary").unwrap();

    fs::create_dir_all(&paths.state_dir).unwrap();
    let archive_path = paths.state_dir.join(format!("{package}-{version}.tar.gz"));
    write_tar_gz(&archive_path, &source_dir, "not-rg");

    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    write_manifest_with_archive(
        paths,
        package,
        version,
        &archive_path,
        &checksum,
        package,
        "tar.gz",
    );
}

pub fn seed_symlink_binary_fixture(paths: &HivePaths, package: &str, version: &str) {
    fs::create_dir_all(&paths.state_dir).unwrap();
    let archive_path = paths.state_dir.join(format!("{package}-{version}.tar.gz"));
    let payload_dir = paths.state_dir.join("fixture-payload");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::write(payload_dir.join("sh"), "stub-binary").unwrap();
    write_tar_gz_with_symlink(&archive_path, "release/bin", &payload_dir);

    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(&archive_path).unwrap())
    );
    write_manifest_with_binaries_with_archive(
        paths,
        package,
        version,
        &archive_path,
        &checksum,
        &["bin/sh"],
        "tar.gz",
    );
}

pub fn seed_installed_package(paths: &HivePaths, package: &str, versions: &[&str], active: &str) {
    for version in versions {
        let install_dir = paths.package_store.join(package).join(version);
        fs::create_dir_all(&install_dir).unwrap();
        let binary_path = install_dir.join(package);
        if let Some(parent) = binary_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(binary_path, format!("binary-{version}")).unwrap();
    }

    let manifest_version = versions.last().unwrap().to_string();
    write_manifest(
        paths,
        package,
        &manifest_version,
        &PathBuf::from("/tmp/unused.tar.gz"),
        "sha256:unused",
        package,
    );

    let store = StateStore::new(paths.state_dir.clone());
    store
        .save_package(&InstalledPackage {
            name: package.into(),
            versions: versions.iter().map(|value| value.to_string()).collect(),
            active: Some(active.into()),
        })
        .unwrap();
}

pub fn seed_installed_package_with_binaries(
    paths: &HivePaths,
    package: &str,
    versions: &[&str],
    active: &str,
    binary_names: &[&str],
) {
    for version in versions {
        let install_dir = paths.package_store.join(package).join(version);
        fs::create_dir_all(&install_dir).unwrap();
        for binary_name in binary_names {
            let binary_path = install_dir.join(binary_name);
            if let Some(parent) = binary_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(binary_path, format!("binary-{version}-{binary_name}")).unwrap();
        }
    }

    let manifest_version = versions.last().unwrap().to_string();
    write_manifest_with_binaries(
        paths,
        package,
        &manifest_version,
        &PathBuf::from("/tmp/unused.tar.gz"),
        "sha256:unused",
        binary_names,
    );

    let store = StateStore::new(paths.state_dir.clone());
    store
        .save_package(&InstalledPackage {
            name: package.into(),
            versions: versions.iter().map(|value| value.to_string()).collect(),
            active: Some(active.into()),
        })
        .unwrap();
}

fn write_manifest(
    paths: &HivePaths,
    package: &str,
    version: &str,
    archive_path: &Path,
    checksum: &str,
    binary_name: &str,
) {
    write_manifest_with_binaries_with_archive(
        paths,
        package,
        version,
        archive_path,
        checksum,
        &[binary_name],
        "tar.gz",
    );
}

pub fn write_manifest_with_binaries(
    paths: &HivePaths,
    package: &str,
    version: &str,
    archive_path: &Path,
    checksum: &str,
    binary_names: &[&str],
) {
    write_manifest_with_binaries_with_archive(
        paths,
        package,
        version,
        archive_path,
        checksum,
        binary_names,
        "tar.gz",
    );
}

fn write_manifest_with_archive(
    paths: &HivePaths,
    package: &str,
    version: &str,
    archive_path: &Path,
    checksum: &str,
    binary_name: &str,
    archive: &str,
) {
    write_manifest_with_binaries_with_archive(
        paths,
        package,
        version,
        archive_path,
        checksum,
        &[binary_name],
        archive,
    );
}

pub fn write_manifest_with_binaries_with_archive(
    paths: &HivePaths,
    package: &str,
    version: &str,
    archive_path: &Path,
    checksum: &str,
    binary_names: &[&str],
    archive: &str,
) {
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join(format!("{package}.toml")),
        format!(
            "name = \"{package}\"\nversion = \"{version}\"\n\n[platform.{platform}]\nurl = \"file://{archive}\"\nchecksum = \"{checksum}\"\narchive = \"{archive_kind}\"\nbinaries = [{binaries}]\n",
            platform = current_platform_key(),
            archive = archive_path.display(),
            archive_kind = archive,
            binaries = binary_names
                .iter()
                .map(|binary_name| format!("\"{binary_name}\""))
                .collect::<Vec<_>>()
                .join(", "),
        ),
    )
    .unwrap();
}

pub fn current_platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        _ => panic!("unsupported test host"),
    }
}

pub fn platform_archive_name(package: &str, version: &str) -> String {
    match current_platform_key() {
        "linux-x86_64" => format!("{package}-{version}-x86_64-unknown-linux-musl.tar.gz"),
        "linux-aarch64" => format!("{package}-{version}-aarch64-unknown-linux-musl.tar.gz"),
        "macos-x86_64" => format!("{package}-{version}-x86_64-apple-darwin.tar.gz"),
        "macos-aarch64" => format!("{package}-{version}-aarch64-apple-darwin.tar.gz"),
        _ => panic!("unsupported test host"),
    }
}

pub fn manifest_with_github_source(
    package: &str,
    version: &str,
    repo: &str,
    channel: &str,
) -> Manifest {
    Manifest {
        name: package.to_string(),
        version: version.to_string(),
        source: Some(ManifestSource {
            github: Some(GitHubSource {
                repo: repo.to_string(),
                channel: channel.to_string(),
                platform: BTreeMap::new(),
            }),
        }),
        platform: BTreeMap::from([(
            current_platform_key().to_string(),
            Artifact {
                url: "https://example.invalid/rg.tar.gz".to_string(),
                checksum: "sha256:abc".to_string(),
                archive: "tar.gz".to_string(),
                binaries: vec![package.to_string()],
            },
        )]),
    }
}

pub fn release_json(
    tag_name: &str,
    prerelease: bool,
    draft: bool,
    assets: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "tag_name": tag_name,
        "prerelease": prerelease,
        "draft": draft,
        "assets": assets,
    })
}

pub fn asset_json(name: &str, browser_download_url: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "browser_download_url": browser_download_url,
    })
}

pub struct MockGitHubServer {
    api_base: String,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl MockGitHubServer {
    pub fn api_base(&self) -> &str {
        &self.api_base
    }
}

impl Drop for MockGitHubServer {
    fn drop(&mut self) {
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().unwrap();
        }
    }
}

pub fn spawn_github_server(releases: Vec<serde_json::Value>) -> MockGitHubServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let api_base = format!("http://{}", listener.local_addr().unwrap());
    let body = serde_json::to_vec(&releases).unwrap();

    let join_handle = thread::spawn(move || {
        for _ in 0..200 {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request).unwrap();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                    stream.flush().unwrap();
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("failed to accept mock GitHub request: {error}"),
            }
        }
    });

    MockGitHubServer {
        api_base,
        join_handle: Some(join_handle),
    }
}

pub struct MockHttpServer {
    url: String,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl MockHttpServer {
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().unwrap();
        }
    }
}

pub fn spawn_http_server(body: Vec<u8>, content_type: &str) -> MockHttpServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let content_type = content_type.to_string();

    let join_handle = thread::spawn(move || {
        for _ in 0..200 {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request).unwrap();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                    stream.flush().unwrap();
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("failed to accept mock HTTP request: {error}"),
            }
        }
    });

    MockHttpServer {
        url,
        join_handle: Some(join_handle),
    }
}

pub struct EnvLock {
    _guard: MutexGuard<'static, ()>,
}

pub fn lock_env() -> EnvLock {
    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    EnvLock {
        _guard: ENV_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap(),
    }
}

pub fn write_named_tar_gz(root: &Path, archive_name: &str, binary_name: &str) -> PathBuf {
    let source_dir = root.join(format!("{archive_name}-source"));
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join(binary_name), "stub-binary").unwrap();

    let archive_path = root.join(archive_name);
    write_tar_gz(&archive_path, &source_dir, binary_name);
    archive_path
}

pub fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

pub fn write_manifest_with_github_source_and_binaries(
    paths: &HivePaths,
    package: &str,
    version: &str,
    repo: &str,
    channel: &str,
    binary_names: &[&str],
) {
    let mut manifest = manifest_with_github_source(package, version, repo, channel);
    manifest
        .platform
        .get_mut(current_platform_key())
        .unwrap()
        .binaries = binary_names.iter().map(|value| value.to_string()).collect();
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join(format!("{package}.toml")),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
}

pub fn write_manifest_with_github_source_and_checksum(
    paths: &HivePaths,
    package: &str,
    version: &str,
    repo: &str,
    channel: &str,
    url: &str,
    checksum: &str,
) {
    let mut manifest = manifest_with_github_source(package, version, repo, channel);
    let artifact = manifest.platform.get_mut(current_platform_key()).unwrap();
    artifact.url = url.to_string();
    artifact.checksum = checksum.to_string();
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join(format!("{package}.toml")),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
}

pub fn write_manifest_with_github_source_platforms(
    paths: &HivePaths,
    package: &str,
    version: &str,
    repo: &str,
    channel: &str,
    platforms: &[(&str, &str, &str, &[&str])],
) {
    let manifest = Manifest {
        name: package.to_string(),
        version: version.to_string(),
        source: Some(ManifestSource {
            github: Some(GitHubSource {
                repo: repo.to_string(),
                channel: channel.to_string(),
                platform: BTreeMap::new(),
            }),
        }),
        platform: platforms
            .iter()
            .map(|(platform, url, checksum, binaries)| {
                (
                    (*platform).to_string(),
                    Artifact {
                        url: (*url).to_string(),
                        checksum: (*checksum).to_string(),
                        archive: "tar.gz".to_string(),
                        binaries: binaries.iter().map(|value| (*value).to_string()).collect(),
                    },
                )
            })
            .collect(),
    };
    fs::create_dir_all(&paths.manifest_dirs[0]).unwrap();
    fs::write(
        paths.manifest_dirs[0].join(format!("{package}.toml")),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
}
