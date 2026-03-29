use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn activate_version(shim_dir: &Path, binaries: &[(String, PathBuf)]) -> Result<(), String> {
    fs::create_dir_all(shim_dir)
        .map_err(|error| format!("failed to create {}: {error}", shim_dir.display()))?;

    for (name, target) in binaries {
        let link = shim_dir.join(name);
        if link.symlink_metadata().is_ok() {
            fs::remove_file(&link)
                .map_err(|error| format!("failed to remove {}: {error}", link.display()))?;
        }
        std::os::unix::fs::symlink(target, &link)
            .map_err(|error| format!("failed to link {}: {error}", link.display()))?;
    }

    Ok(())
}
