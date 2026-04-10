#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use photoframe_app::{ImageArtifact, ImageFormat};
use serde::{Deserialize, Serialize};

const PHOTO_HISTORY_DIR_NAME: &str = "pfphotos";
const PHOTO_HISTORY_INDEX_FILE: &str = "index.json";
pub(crate) const PHOTO_HISTORY_MAX_ENTRIES: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CachedPhotoEntry {
    pub(crate) sha256: String,
    pub(crate) format: String,
    pub(crate) created_epoch: i64,
    pub(crate) file_name: String,
    #[serde(default)]
    pub(crate) image_date: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedPhotoIndex {
    entries: Vec<CachedPhotoEntry>,
}

fn current_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn history_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("PHOTOFRAME_PHOTO_HISTORY_DIR") {
        return PathBuf::from(path);
    }
    PathBuf::from(crate::sdcard::mount_path()).join(PHOTO_HISTORY_DIR_NAME)
}

fn index_path() -> PathBuf {
    history_dir().join(PHOTO_HISTORY_INDEX_FILE)
}

fn entry_path(file_name: &str) -> PathBuf {
    history_dir().join(file_name)
}

fn ensure_history_dir() -> Result<PathBuf, String> {
    let dir = history_dir();
    fs::create_dir_all(&dir)
        .map_err(|err| format!("create photo history dir {} failed: {err}", dir.display()))?;
    Ok(dir)
}

fn load_index() -> Result<CachedPhotoIndex, String> {
    let path = index_path();
    if !path.exists() {
        return Ok(CachedPhotoIndex::default());
    }
    let bytes = fs::read(&path)
        .map_err(|err| format!("read photo history index {} failed: {err}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| format!("parse photo history index {} failed: {err}", path.display()))
}

fn save_index(index: &CachedPhotoIndex) -> Result<(), String> {
    ensure_history_dir()?;
    let path = index_path();
    let bytes = serde_json::to_vec(index).map_err(|err| {
        format!(
            "serialize photo history index {} failed: {err}",
            path.display()
        )
    })?;
    fs::write(&path, bytes)
        .map_err(|err| format!("write photo history index {} failed: {err}", path.display()))
}

fn format_ext(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Bmp => "bmp",
        ImageFormat::Jpeg => "jpg",
    }
}

fn format_label(format: &ImageFormat) -> &'static str {
    match format {
        ImageFormat::Bmp => "bmp",
        ImageFormat::Jpeg => "jpeg",
    }
}

fn parse_format(label: &str) -> Option<ImageFormat> {
    match label {
        "bmp" => Some(ImageFormat::Bmp),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        _ => None,
    }
}

#[cfg(test)]
fn next_entry_from_slice<'a>(
    entries: &'a [CachedPhotoEntry],
    current_sha256: &str,
) -> Option<&'a CachedPhotoEntry> {
    if entries.is_empty() {
        return None;
    }
    if current_sha256.is_empty() {
        return entries.first();
    }
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.sha256 == current_sha256)
    {
        return entries.get((index + 1) % entries.len());
    }
    entries.first()
}

pub(crate) fn remember_rendered_photo(
    artifact: &ImageArtifact,
    sha256: &str,
    image_date: Option<&str>,
) -> Result<(), String> {
    if !crate::sdcard::is_ready() {
        return Err("sdcard not ready".into());
    }
    let sha256 = sha256.trim();
    if sha256.is_empty() {
        return Err("photo sha256 is empty".into());
    }

    let dir = ensure_history_dir()?;
    let mut index = load_index()?;
    let file_name = format!("{sha256}.{}", format_ext(&artifact.format));
    let path = dir.join(&file_name);
    fs::write(&path, &artifact.bytes)
        .map_err(|err| format!("write cached photo {} failed: {err}", path.display()))?;

    let normalized_date = image_date.unwrap_or_default().trim().to_string();
    let replace_index = if normalized_date.is_empty() {
        index
            .entries
            .iter()
            .position(|entry| entry.sha256 == sha256)
    } else {
        index
            .entries
            .iter()
            .position(|entry| entry.image_date == normalized_date)
    };
    if let Some(existing) = replace_index {
        let previous = index.entries.remove(existing);
        if previous.file_name != file_name
            && !index
                .entries
                .iter()
                .any(|entry| entry.file_name == previous.file_name)
        {
            let previous_path = entry_path(&previous.file_name);
            let _ = fs::remove_file(previous_path);
        }
    }

    index.entries.insert(
        0,
        CachedPhotoEntry {
            sha256: sha256.to_string(),
            format: format_label(&artifact.format).to_string(),
            created_epoch: current_epoch(),
            file_name: file_name.clone(),
            image_date: normalized_date,
        },
    );

    let removed = index.entries.split_off(PHOTO_HISTORY_MAX_ENTRIES);
    for entry in removed {
        if !index
            .entries
            .iter()
            .any(|remaining| remaining.file_name == entry.file_name)
        {
            let _ = fs::remove_file(entry_path(&entry.file_name));
        }
    }

    save_index(&index)
}

pub(crate) fn load_artifact_by_sha256(sha256: &str) -> Result<Option<ImageArtifact>, String> {
    let Some(entry) = entry_for_sha256(sha256)? else {
        return Ok(None);
    };
    let Some(format) = parse_format(&entry.format) else {
        return Err(format!(
            "unsupported cached photo format={} sha256={}",
            entry.format, entry.sha256
        ));
    };
    let path = entry_path(&entry.file_name);
    let bytes = fs::read(&path)
        .map_err(|err| format!("read cached photo {} failed: {err}", path.display()))?;
    Ok(Some(ImageArtifact {
        format,
        width: 0,
        height: 0,
        bytes,
    }))
}

pub(crate) fn entry_for_sha256(sha256: &str) -> Result<Option<CachedPhotoEntry>, String> {
    let sha256 = sha256.trim();
    if sha256.is_empty() {
        return Ok(None);
    }
    let index = load_index()?;
    Ok(index
        .entries
        .into_iter()
        .find(|entry| entry.sha256 == sha256))
}

pub(crate) fn entry_for_date(date_text: &str) -> Result<Option<CachedPhotoEntry>, String> {
    let date_text = date_text.trim();
    if date_text.is_empty() {
        return Ok(None);
    }
    let index = load_index()?;
    Ok(index
        .entries
        .into_iter()
        .find(|entry| entry.image_date == date_text))
}

#[cfg(test)]
pub(crate) fn next_entry(current_sha256: &str) -> Result<Option<CachedPhotoEntry>, String> {
    let index = load_index()?;
    Ok(next_entry_from_slice(&index.entries, current_sha256).cloned())
}

#[cfg(test)]
fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::{
        PHOTO_HISTORY_MAX_ENTRIES, entry_for_date, entry_for_sha256, history_dir,
        load_artifact_by_sha256, next_entry, remember_rendered_photo, test_lock,
    };
    use photoframe_app::{ImageArtifact, ImageFormat};
    use std::{
        fs,
        sync::MutexGuard,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn artifact(tag: &str, format: ImageFormat) -> ImageArtifact {
        ImageArtifact {
            format,
            width: 0,
            height: 0,
            bytes: tag.as_bytes().to_vec(),
        }
    }

    fn setup_env() -> (MutexGuard<'static, ()>, String, std::path::PathBuf) {
        let guard = test_lock().lock().unwrap();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pf-photo-history-{unique}"));
        unsafe {
            std::env::set_var("PHOTOFRAME_PHOTO_HISTORY_DIR", &root);
        }
        crate::sdcard::set_test_ready(true);
        (guard, "PHOTOFRAME_PHOTO_HISTORY_DIR".into(), root)
    }

    fn cleanup_env(_guard: MutexGuard<'static, ()>, env_key: String, root: std::path::PathBuf) {
        crate::sdcard::set_test_ready(false);
        unsafe {
            std::env::remove_var(env_key);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remember_photo_keeps_latest_entries_and_loads_back() {
        let (guard, env_key, root) = setup_env();

        remember_rendered_photo(
            &artifact("first", ImageFormat::Bmp),
            "sha-1",
            Some("2026-04-10"),
        )
        .unwrap();
        remember_rendered_photo(
            &artifact("second", ImageFormat::Jpeg),
            "sha-2",
            Some("2026-04-09"),
        )
        .unwrap();

        let newest = next_entry("").unwrap().unwrap();
        assert_eq!(newest.sha256, "sha-2");
        assert_eq!(newest.image_date, "2026-04-09");
        assert_eq!(history_dir(), root);

        let loaded = load_artifact_by_sha256("sha-1").unwrap().unwrap();
        assert_eq!(loaded.format, ImageFormat::Bmp);
        assert_eq!(loaded.bytes, b"first");
        assert_eq!(
            entry_for_date("2026-04-10").unwrap().unwrap().sha256,
            "sha-1"
        );

        cleanup_env(guard, env_key, root);
    }

    #[test]
    fn next_entry_cycles_through_current_and_previous_items() {
        let (guard, env_key, root) = setup_env();

        remember_rendered_photo(&artifact("first", ImageFormat::Bmp), "sha-1", None).unwrap();
        remember_rendered_photo(&artifact("second", ImageFormat::Bmp), "sha-2", None).unwrap();
        remember_rendered_photo(&artifact("third", ImageFormat::Bmp), "sha-3", None).unwrap();

        assert_eq!(next_entry("sha-3").unwrap().unwrap().sha256, "sha-2");
        assert_eq!(next_entry("sha-2").unwrap().unwrap().sha256, "sha-1");
        assert_eq!(next_entry("sha-1").unwrap().unwrap().sha256, "sha-3");

        cleanup_env(guard, env_key, root);
    }

    #[test]
    fn remember_photo_caps_history_length() {
        let (guard, env_key, root) = setup_env();

        for idx in 0..(PHOTO_HISTORY_MAX_ENTRIES + 3) {
            let sha = format!("sha-{idx}");
            remember_rendered_photo(&artifact(&sha, ImageFormat::Bmp), &sha, None).unwrap();
        }

        assert!(entry_for_sha256("sha-0").unwrap().is_none());
        assert!(entry_for_sha256("sha-1").unwrap().is_none());
        assert!(entry_for_sha256("sha-2").unwrap().is_none());
        assert!(entry_for_sha256("sha-3").unwrap().is_some());

        cleanup_env(guard, env_key, root);
    }

    #[test]
    fn same_sha_can_back_multiple_dates_without_losing_current_file() {
        let (guard, env_key, root) = setup_env();

        remember_rendered_photo(
            &artifact("same", ImageFormat::Bmp),
            "sha-same",
            Some("2026-04-10"),
        )
        .unwrap();
        remember_rendered_photo(
            &artifact("same", ImageFormat::Bmp),
            "sha-same",
            Some("2026-04-09"),
        )
        .unwrap();

        assert_eq!(
            entry_for_date("2026-04-10").unwrap().unwrap().sha256,
            "sha-same"
        );
        assert_eq!(
            entry_for_date("2026-04-09").unwrap().unwrap().sha256,
            "sha-same"
        );
        assert!(history_dir().join("sha-same.bmp").exists());

        cleanup_env(guard, env_key, root);
    }
}
