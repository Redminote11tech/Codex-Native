use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PICKLE_HEADER_OFFSET: usize = 16;

#[derive(Debug)]
pub enum AsarError {
    Io(io::Error),
    Json(serde_json::Error),
    InvalidFormat(&'static str),
    InvalidOffset(String),
    MissingEntry(String),
}

impl Display for AsarError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "i/o error: {error}"),
            Self::Json(error) => write!(f, "json parse error: {error}"),
            Self::InvalidFormat(message) => write!(f, "invalid asar format: {message}"),
            Self::InvalidOffset(offset) => write!(f, "invalid asar entry offset: {offset}"),
            Self::MissingEntry(path) => write!(f, "missing asar entry: {path}"),
        }
    }
}

impl std::error::Error for AsarError {}

impl From<io::Error> for AsarError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for AsarError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Debug, Deserialize)]
pub struct Header {
    pub files: BTreeMap<String, Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub files: BTreeMap<String, Entry>,
    #[serde(default)]
    pub offset: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub executable: Option<bool>,
    #[serde(default)]
    pub unpacked: Option<bool>,
}

#[derive(Debug)]
pub struct AsarArchive {
    bytes: Vec<u8>,
    payload_offset: usize,
    header: Header,
}

impl AsarArchive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AsarError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, AsarError> {
        if bytes.len() < PICKLE_HEADER_OFFSET {
            return Err(AsarError::InvalidFormat("file is shorter than ASAR header"));
        }

        let header_size = read_u32_le(&bytes, 12)? as usize;
        let header_end = PICKLE_HEADER_OFFSET
            .checked_add(header_size)
            .ok_or(AsarError::InvalidFormat("header size overflow"))?;

        if header_end > bytes.len() {
            return Err(AsarError::InvalidFormat("header extends past end of file"));
        }

        let header = serde_json::from_slice::<Header>(&bytes[PICKLE_HEADER_OFFSET..header_end])?;

        Ok(Self {
            bytes,
            payload_offset: header_end,
            header,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn entry(&self, path: &str) -> Result<&Entry, AsarError> {
        let mut files = &self.header.files;
        let mut current = None;

        for component in path.split('/').filter(|component| !component.is_empty()) {
            let entry = files
                .get(component)
                .ok_or_else(|| AsarError::MissingEntry(path.to_string()))?;
            current = Some(entry);
            files = &entry.files;
        }

        current.ok_or_else(|| AsarError::MissingEntry(path.to_string()))
    }

    pub fn read_file(&self, path: &str) -> Result<&[u8], AsarError> {
        let entry = self.entry(path)?;
        let size = entry
            .size
            .ok_or(AsarError::InvalidFormat("file entry missing size"))? as usize;
        let offset = entry
            .offset
            .as_deref()
            .ok_or(AsarError::InvalidFormat("file entry missing offset"))?;
        let offset = offset
            .parse::<usize>()
            .map_err(|_| AsarError::InvalidOffset(offset.to_string()))?;

        let start = self
            .payload_offset
            .checked_add(offset)
            .ok_or(AsarError::InvalidFormat("file start overflow"))?;
        let end = start
            .checked_add(size)
            .ok_or(AsarError::InvalidFormat("file end overflow"))?;

        if end > self.bytes.len() {
            return Err(AsarError::InvalidFormat("file entry extends past end of archive"));
        }

        Ok(&self.bytes[start..end])
    }

    pub fn list_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        collect_paths(&self.header.files, Path::new(""), &mut files);
        files
    }

    pub fn extract_all(&self, output_dir: impl AsRef<Path>) -> Result<(), AsarError> {
        for file in self.list_files() {
            let entry = self.entry(&file)?;
            let target = output_dir.as_ref().join(&file);

            if entry.unpacked.unwrap_or(false) {
                continue;
            }

            if !entry.files.is_empty() && entry.size.is_none() {
                fs::create_dir_all(&target)?;
                continue;
            }

            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::write(&target, self.read_file(&file)?)?;

            #[cfg(unix)]
            if entry.executable.unwrap_or(false) {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&target)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target, perms)?;
            }
        }

        Ok(())
    }
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, AsarError> {
    let range_end = offset
        .checked_add(4)
        .ok_or(AsarError::InvalidFormat("u32 offset overflow"))?;
    let window = bytes
        .get(offset..range_end)
        .ok_or(AsarError::InvalidFormat("u32 read past end of file"))?;
    let array: [u8; 4] = window
        .try_into()
        .map_err(|_| AsarError::InvalidFormat("failed to parse u32"))?;
    Ok(u32::from_le_bytes(array))
}

fn collect_paths(entries: &BTreeMap<String, Entry>, prefix: &Path, files: &mut Vec<String>) {
    for (name, entry) in entries {
        let path = prefix.join(name);

        if entry.files.is_empty() {
            files.push(path_to_unix_string(&path));
            continue;
        }

        collect_paths(&entry.files, &path, files);
    }
}

fn path_to_unix_string(path: &Path) -> String {
    let mut converted = PathBuf::new();
    converted.push(path);
    converted.to_string_lossy().replace('\\', "/")
}
