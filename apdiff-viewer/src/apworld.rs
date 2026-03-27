use std::io::Read;
use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

#[derive(Debug, PartialEq)]
pub enum FileContent {
    Text(String),
    Binary([u8; 32]),
}

pub type FileTree = BTreeMap<PathBuf, FileContent>;

const MAX_ENTRY_SIZE: u64 = 10 * 1024 * 1024;
const MAX_TOTAL_SIZE: u64 = 100 * 1024 * 1024;
const MAX_ENTRY_COUNT: usize = 10_000;

pub fn extract_apworld(data: &[u8]) -> Result<FileTree> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = ZipArchive::new(cursor)?;

    if archive.len() > MAX_ENTRY_COUNT {
        bail!(
            "Apworld has too many entries ({}, max {})",
            archive.len(),
            MAX_ENTRY_COUNT
        );
    }

    let mut tree = BTreeMap::new();
    let mut total_size: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;

        if entry.is_dir() {
            continue;
        }

        if entry.is_symlink() {
            bail!("Apworld contains a symlink: {:?}", entry.name());
        }

        let path = entry
            .enclosed_name()
            .ok_or_else(|| anyhow::anyhow!("Path traversal in apworld entry: {}", entry.name()))?;

        // Read with a hard limit to prevent zip bombs — entry.size() is
        // from the zip header and can lie, so we limit actual decompressed bytes.
        let mut buf = Vec::new();
        let bytes_read = entry
            .take(MAX_ENTRY_SIZE + 1)
            .read_to_end(&mut buf)
            .with_context(|| format!("Failed to read entry {path:?}"))?;

        if bytes_read as u64 > MAX_ENTRY_SIZE {
            bail!(
                "Entry {} too large (>{} bytes)",
                path.display(),
                MAX_ENTRY_SIZE
            );
        }

        total_size += bytes_read as u64;
        if total_size > MAX_TOTAL_SIZE {
            bail!(
                "Total extracted size exceeds limit ({} bytes)",
                MAX_TOTAL_SIZE
            );
        }

        let content = match String::from_utf8(buf) {
            Ok(text) => FileContent::Text(text),
            Err(e) => FileContent::Binary(Sha256::digest(&e.into_bytes()).into()),
        };

        tree.insert(path, content);
    }

    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Vec::new();
        let mut writer = ZipWriter::new(std::io::Cursor::new(buf));
        for (name, content) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_basic_extraction() {
        let zip = make_zip(&[
            ("hello.py", b"print('hello')"),
            ("data/items.json", b"{\"items\": []}"),
        ]);
        let tree = extract_apworld(&zip).unwrap();
        assert_eq!(tree.len(), 2);
        assert!(
            matches!(tree[Path::new("hello.py")], FileContent::Text(ref s) if s == "print('hello')")
        );
        assert!(matches!(
            tree[Path::new("data/items.json")],
            FileContent::Text(_)
        ));
    }

    #[test]
    fn test_binary_detection() {
        let zip = make_zip(&[("image.png", &[0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF])]);
        let tree = extract_apworld(&zip).unwrap();
        assert!(matches!(
            tree[Path::new("image.png")],
            FileContent::Binary(_)
        ));
    }

    #[test]
    fn test_skips_directories() {
        let buf = Vec::new();
        let mut writer = ZipWriter::new(std::io::Cursor::new(buf));
        writer
            .add_directory("somedir/", SimpleFileOptions::default())
            .unwrap();
        writer
            .start_file("somedir/file.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"content").unwrap();
        let zip = writer.finish().unwrap().into_inner();

        let tree = extract_apworld(&zip).unwrap();
        assert_eq!(tree.len(), 1);
        assert!(tree.contains_key(Path::new("somedir/file.txt")));
    }

    #[test]
    fn test_entry_count_limit() {
        let buf = Vec::new();
        let mut writer = ZipWriter::new(std::io::Cursor::new(buf));
        for i in 0..MAX_ENTRY_COUNT + 1 {
            writer
                .start_file(format!("file_{i}.txt"), SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        let zip = writer.finish().unwrap().into_inner();

        let result = extract_apworld(&zip);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too many entries"));
    }

    #[test]
    fn test_empty_zip() {
        let buf = Vec::new();
        let writer = ZipWriter::new(std::io::Cursor::new(buf));
        let zip = writer.finish().unwrap().into_inner();
        let tree = extract_apworld(&zip).unwrap();
        assert!(tree.is_empty());
    }
}
