use anyhow::Result;
use std::path::Path;

/// Copy the content of a directory `src` into `dst`. `dst` must be a directory.
pub(crate) fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), &dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}


/// Copy a file or directory from `src` to `dst`. This will replace `dst` if it exists.
pub(crate) fn copy_file_or_dir(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        delete_file_or_dir(dst)?;
    }

    if src.is_dir() {
        copy_dir_all(&src, &dst)?;
    } else if src.is_file() {
        std::fs::copy(&src, &dst)?;
    }

    Ok(())
}

/// Delete a directory or file at `path`.
pub(crate) fn delete_file_or_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else if path.is_file() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

