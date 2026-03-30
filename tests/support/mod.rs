#![allow(dead_code)]

use flate2::{Compression, write::GzEncoder};
use hive::{
    config::HivePaths,
    state::{InstalledPackage, StateStore},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io,
    path::{Path, PathBuf},
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

pub fn write_tar_gz_with_wrapper(
    archive_path: &Path,
    source_dir: &Path,
    wrapper_dir: &str,
) {
    let tar_gz = fs::File::create(archive_path).unwrap();
    let encoder = GzEncoder::new(tar_gz, Compression::default());
    let mut builder = Builder::new(encoder);
    builder.append_dir_all(wrapper_dir, source_dir).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

pub fn write_tar_gz_with_symlink(
    archive_path: &Path,
    link_path: &str,
    link_target: &Path,
) {
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
    write_manifest_with_archive(paths, package, version, &archive_path, &checksum, package, "tar.gz");
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
    write_manifest_with_archive(paths, package, version, &archive_path, &checksum, package, "tar.xz");
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
    write_manifest_with_archive(paths, package, version, &archive_path, &checksum, package, "tar.gz");
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

fn write_manifest_with_binaries(
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

fn write_manifest_with_binaries_with_archive(
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

fn current_platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        _ => panic!("unsupported test host"),
    }
}
