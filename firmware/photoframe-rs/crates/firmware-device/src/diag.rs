#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "espidf")]
use std::{ffi::CString, os::unix::ffi::OsStrExt};
#[cfg(not(target_os = "espidf"))]
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
};

use photoframe_app::{DeviceRuntimeConfig, LogUploadProvider};
use photoframe_contracts::{DeviceLogUploadRequest, DeviceLogUploadRequestBody};

const LOG_BUFFER_MAX_BYTES: usize = 64 * 1024;
const LOG_BUFFER_MAX_LINES: usize = 800;
const LOG_LINE_MAX_CHARS: usize = 320;

const TF_LOG_TOTAL_BYTES: usize = 10 * 1024 * 1024;
const TF_LOG_SEGMENT_COUNT: usize = 20;
const TF_LOG_SEGMENT_CAP_BYTES: usize = TF_LOG_TOTAL_BYTES / TF_LOG_SEGMENT_COUNT;
const TF_LOG_BLOCK_MAGIC: u32 = 0x5046_4C42;
const TF_LOG_BLOCK_VERSION: u16 = 1;
const TF_LOG_BLOCK_HEADER_BYTES: usize = 40;

const PERSISTED_LOG_MAGIC: u32 = 0x5046_4C47;
const PERSISTED_LOG_VERSION: u32 = 1;
const PERSISTED_LOG_MAX_BYTES: usize = 7 * 1024;

#[derive(Clone, Debug, Default)]
struct RestoredLogSnapshot {
    boot_id: u32,
    line_count: usize,
    truncated: bool,
    lines: Vec<String>,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct PersistedLogSnapshot {
    magic: u32,
    version: u32,
    boot_id: u32,
    used_bytes: u32,
    line_count: u32,
    truncated: u32,
    bytes: [u8; PERSISTED_LOG_MAX_BYTES],
}

impl PersistedLogSnapshot {
    const fn empty() -> Self {
        Self {
            magic: 0,
            version: PERSISTED_LOG_VERSION,
            boot_id: 0,
            used_bytes: 0,
            line_count: 0,
            truncated: 0,
            bytes: [0; PERSISTED_LOG_MAX_BYTES],
        }
    }
}

#[cfg(target_os = "espidf")]
#[unsafe(link_section = ".rtc.data")]
static mut PERSISTED_LOG_SNAPSHOT: PersistedLogSnapshot = PersistedLogSnapshot::empty();

#[cfg(not(target_os = "espidf"))]
static mut PERSISTED_LOG_SNAPSHOT: PersistedLogSnapshot = PersistedLogSnapshot::empty();

#[derive(Default)]
struct LogBuffer {
    boot_id: u32,
    next_seq: u64,
    total_bytes: usize,
    restored_prefix_lines: usize,
    current_boot_truncated: bool,
    lines: VecDeque<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct TfLogBlock {
    segment_index: usize,
    generation: u64,
    boot_id: u32,
    line_count: usize,
    truncated: bool,
    created_epoch: i64,
    text_bytes: usize,
    end_offset: usize,
    lines: Vec<String>,
}

impl LogBuffer {
    fn start_boot(&mut self, next_boot_id: u32, restored: Option<&RestoredLogSnapshot>) {
        self.boot_id = next_boot_id;
        self.next_seq = 0;
        self.total_bytes = 0;
        self.restored_prefix_lines = 0;
        self.current_boot_truncated = false;
        self.lines.clear();
        if let Some(snapshot) = restored {
            for line in snapshot.lines.iter() {
                self.push_serialized_line(line.clone());
            }
            self.restored_prefix_lines = snapshot.lines.len().min(self.lines.len());
        }
    }

    fn push(&mut self, level: &str, message: &str) {
        let epoch = current_epoch();
        self.next_seq = self.next_seq.wrapping_add(1);
        let body: String = message.chars().take(LOG_LINE_MAX_CHARS).collect();
        let line = format!(
            "[{}][boot:{}][seq:{}][{}] {}",
            epoch, self.boot_id, self.next_seq, level, body
        );
        self.push_serialized_line(line);
    }

    fn push_serialized_line(&mut self, line: String) {
        self.total_bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > LOG_BUFFER_MAX_LINES || self.total_bytes > LOG_BUFFER_MAX_BYTES {
            if let Some(removed) = self.lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(removed.len());
                if self.restored_prefix_lines > 0 {
                    self.restored_prefix_lines = self.restored_prefix_lines.saturating_sub(1);
                } else {
                    self.current_boot_truncated = true;
                }
            } else {
                self.total_bytes = 0;
                break;
            }
        }
    }

    fn snapshot_for_rtc(&self) -> RestoredLogSnapshot {
        let mut selected: Vec<String> = Vec::new();
        let mut selected_bytes = 0usize;

        for line in self.lines.iter().rev() {
            let line_bytes = line.len();
            let separator_bytes = usize::from(!selected.is_empty());
            if !selected.is_empty()
                && selected_bytes + separator_bytes + line_bytes > PERSISTED_LOG_MAX_BYTES
            {
                break;
            }
            if selected.is_empty() && line_bytes > PERSISTED_LOG_MAX_BYTES {
                let clipped: String = line.chars().take(PERSISTED_LOG_MAX_BYTES).collect();
                selected.push(clipped);
                break;
            }
            selected.push(line.clone());
            selected_bytes += separator_bytes + line_bytes;
        }
        selected.reverse();

        RestoredLogSnapshot {
            boot_id: self.boot_id,
            line_count: selected.len(),
            truncated: selected.len() < self.lines.len(),
            lines: selected,
        }
    }

    fn snapshot_for_tf(&self) -> Option<RestoredLogSnapshot> {
        if self.lines.len() <= self.restored_prefix_lines {
            return None;
        }
        let lines = self
            .lines
            .iter()
            .skip(self.restored_prefix_lines)
            .cloned()
            .collect::<Vec<_>>();
        Some(RestoredLogSnapshot {
            boot_id: self.boot_id,
            line_count: lines.len(),
            truncated: self.current_boot_truncated,
            lines,
        })
    }

    fn all_lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }
}

fn current_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn tf_log_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("PHOTOFRAME_TF_LOG_DIR") {
        return PathBuf::from(path);
    }
    PathBuf::from(crate::sdcard::mount_path())
}

fn tf_segment_path(index: usize) -> PathBuf {
    tf_log_dir().join(format!("pflog{index:02}.bin"))
}

fn push_recent_lines(
    selected: &mut Vec<String>,
    selected_bytes: &mut usize,
    lines: &[String],
    max_lines: usize,
    max_bytes: usize,
) {
    for line in lines.iter().rev() {
        let line_bytes = line.len();
        if selected.len() >= max_lines {
            break;
        }
        if !selected.is_empty() && *selected_bytes + line_bytes > max_bytes {
            break;
        }
        if selected.is_empty() && line_bytes > max_bytes {
            let clipped: String = line.chars().take(LOG_LINE_MAX_CHARS.min(max_bytes)).collect();
            *selected_bytes = clipped.len();
            selected.push(clipped);
            break;
        }
        selected.push(line.clone());
        *selected_bytes += line_bytes;
    }
}

fn encode_tf_block(snapshot: &RestoredLogSnapshot, generation: u64, created_epoch: i64) -> Vec<u8> {
    let payload = snapshot.lines.join("\n");
    let payload_bytes = payload.as_bytes();
    let mut encoded = Vec::with_capacity(TF_LOG_BLOCK_HEADER_BYTES + payload_bytes.len());
    encoded.extend_from_slice(&TF_LOG_BLOCK_MAGIC.to_le_bytes());
    encoded.extend_from_slice(&TF_LOG_BLOCK_VERSION.to_le_bytes());
    encoded.extend_from_slice(&0u16.to_le_bytes());
    encoded.extend_from_slice(&generation.to_le_bytes());
    encoded.extend_from_slice(&snapshot.boot_id.to_le_bytes());
    encoded.extend_from_slice(&(snapshot.line_count as u32).to_le_bytes());
    encoded.extend_from_slice(&(payload_bytes.len() as u32).to_le_bytes());
    encoded.extend_from_slice(&u32::from(snapshot.truncated).to_le_bytes());
    encoded.extend_from_slice(&created_epoch.to_le_bytes());
    encoded.extend_from_slice(payload_bytes);
    encoded
}

fn decode_tf_block(data: &[u8], segment_index: usize, offset: usize) -> Option<TfLogBlock> {
    if data.len() < TF_LOG_BLOCK_HEADER_BYTES {
        return None;
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
    let version = u16::from_le_bytes(data[4..6].try_into().ok()?);
    if magic != TF_LOG_BLOCK_MAGIC || version != TF_LOG_BLOCK_VERSION {
        return None;
    }
    let generation = u64::from_le_bytes(data[8..16].try_into().ok()?);
    let boot_id = u32::from_le_bytes(data[16..20].try_into().ok()?);
    let line_count = u32::from_le_bytes(data[20..24].try_into().ok()?);
    let text_bytes = u32::from_le_bytes(data[24..28].try_into().ok()?);
    let truncated = u32::from_le_bytes(data[28..32].try_into().ok()?);
    let created_epoch = i64::from_le_bytes(data[32..40].try_into().ok()?);
    let block_len = TF_LOG_BLOCK_HEADER_BYTES.checked_add(text_bytes as usize)?;
    if block_len > data.len() {
        return None;
    }
    let end_offset = offset.checked_add(block_len)?;
    if end_offset > TF_LOG_SEGMENT_CAP_BYTES {
        return None;
    }
    let payload = String::from_utf8_lossy(&data[TF_LOG_BLOCK_HEADER_BYTES..TF_LOG_BLOCK_HEADER_BYTES + text_bytes as usize]);
    let lines = if payload.is_empty() {
        Vec::new()
    } else {
        payload.lines().map(|line| line.to_string()).collect::<Vec<_>>()
    };
    Some(TfLogBlock {
        segment_index,
        generation,
        boot_id,
        line_count: line_count as usize,
        truncated: truncated != 0,
        created_epoch,
        text_bytes: text_bytes as usize,
        end_offset,
        lines,
    })
}

#[cfg(target_os = "espidf")]
fn tf_path_cstring(path: &PathBuf) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("tf path contains nul byte: {}", path.display()))
}

#[cfg(target_os = "espidf")]
fn clear_tf_segment(path: &PathBuf) -> Result<(), String> {
    let path_c = tf_path_cstring(path)?;
    let unlink_rc = unsafe { libc::unlink(path_c.as_ptr()) };
    if unlink_rc == 0 {
        return Ok(());
    }

    let unlink_err = std::io::Error::last_os_error();
    if unlink_err.kind() == std::io::ErrorKind::NotFound {
        return Ok(());
    }

    let file = unsafe { libc::fopen(path_c.as_ptr(), c"wb".as_ptr()) };
    if file.is_null() {
        return Err(format!("reset tf log segment {} failed: {}", path.display(), unlink_err));
    }
    let fflush_rc = unsafe { libc::fflush(file) };
    let fd = unsafe { libc::fileno(file) };
    let fsync_rc = if fd >= 0 { unsafe { libc::fsync(fd) } } else { 0 };
    let close_rc = unsafe { libc::fclose(file) };
    if fflush_rc != 0 || fsync_rc != 0 || close_rc != 0 {
        return Err(format!(
            "reset tf log segment {} failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "espidf"))]
fn clear_tf_segment(path: &PathBuf) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|err| format!("reset tf log segment {} failed: {err}", path.display()))?;
    file.sync_all()
        .map_err(|err| format!("sync reset tf log segment {} failed: {err}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "espidf")]
fn read_tf_segment_file(path: &PathBuf) -> Result<Option<Vec<u8>>, String> {
    let path_c = tf_path_cstring(path)?;
    let file = unsafe { libc::fopen(path_c.as_ptr(), c"rb".as_ptr()) };
    if file.is_null() {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("open tf log segment {} failed: {err}", path.display()));
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let read_len = unsafe { libc::fread(buf.as_mut_ptr().cast(), 1, buf.len(), file) };
        if read_len > 0 {
            data.extend_from_slice(&buf[..read_len]);
            if data.len() > TF_LOG_SEGMENT_CAP_BYTES {
                unsafe {
                    libc::fclose(file);
                }
                return Err(format!(
                    "tf log segment {} too large: {} > {}",
                    path.display(),
                    data.len(),
                    TF_LOG_SEGMENT_CAP_BYTES
                ));
            }
        }
        if read_len < buf.len() {
            let read_done = unsafe { libc::feof(file) } != 0;
            let read_error = unsafe { libc::ferror(file) } != 0;
            let close_rc = unsafe { libc::fclose(file) };
            if close_rc != 0 {
                return Err(format!(
                    "close tf log segment {} failed: {}",
                    path.display(),
                    std::io::Error::last_os_error()
                ));
            }
            if read_done && !read_error {
                return Ok(Some(data));
            }
            return Err(format!(
                "read tf log segment {} failed: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
    }
}

#[cfg(not(target_os = "espidf"))]
fn read_tf_segment_file(path: &PathBuf) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "stat tf log segment {} failed: {err}",
                path.display()
            ));
        }
    };
    if !metadata.is_file() {
        return Err(format!("tf log segment {} is not a file", path.display()));
    }

    let file_len = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if file_len == 0 {
        return Ok(Some(Vec::new()));
    }
    if file_len > TF_LOG_SEGMENT_CAP_BYTES {
        return Err(format!(
            "tf log segment {} too large: {} > {}",
            path.display(),
            file_len,
            TF_LOG_SEGMENT_CAP_BYTES
        ));
    }

    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|err| format!("open tf log segment {} failed: {err}", path.display()))?;
    let mut data = Vec::with_capacity(file_len);
    file.read_to_end(&mut data)
        .map_err(|err| format!("read tf log segment {} failed: {err}", path.display()))?;
    Ok(Some(data))
}

#[cfg(target_os = "espidf")]
fn write_tf_segment_file(path: &PathBuf, offset: usize, encoded: &[u8]) -> Result<(), String> {
    let path_c = tf_path_cstring(path)?;
    let file = unsafe {
        if offset == 0 {
            libc::fopen(path_c.as_ptr(), c"wb".as_ptr())
        } else {
            libc::fopen(path_c.as_ptr(), c"r+b".as_ptr())
        }
    };
    if file.is_null() {
        return Err(format!(
            "open tf log segment {} failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }

    if offset > 0 {
        let fd = unsafe { libc::fileno(file) };
        if fd < 0 {
            unsafe {
                libc::fclose(file);
            }
            return Err(format!("fileno tf log segment {} failed", path.display()));
        }
        if unsafe { libc::ftruncate(fd, offset as libc::off_t) } != 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::fclose(file);
            }
            return Err(format!(
                "trim tf log segment {} failed: {}",
                path.display(),
                err
            ));
        }
        if unsafe { libc::fseek(file, offset as libc::c_long, libc::SEEK_SET) } != 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::fclose(file);
            }
            return Err(format!(
                "seek tf log segment {} failed: {}",
                path.display(),
                err
            ));
        }
    }

    let written = unsafe { libc::fwrite(encoded.as_ptr().cast(), 1, encoded.len(), file) };
    if written != encoded.len() {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::fclose(file);
        }
        return Err(format!(
            "write tf log segment {} failed: {}",
            path.display(),
            err
        ));
    }
    if unsafe { libc::fflush(file) } != 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::fclose(file);
        }
        return Err(format!(
            "flush tf log segment {} failed: {}",
            path.display(),
            err
        ));
    }
    let fd = unsafe { libc::fileno(file) };
    if fd >= 0 && unsafe { libc::fsync(fd) } != 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::fclose(file);
        }
        return Err(format!(
            "sync tf log segment {} failed: {}",
            path.display(),
            err
        ));
    }
    if unsafe { libc::fclose(file) } != 0 {
        return Err(format!(
            "close tf log segment {} failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "espidf"))]
fn write_tf_segment_file(path: &PathBuf, offset: usize, encoded: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .map_err(|err| format!("open tf log segment {} failed: {err}", path.display()))?;
    if offset == 0 {
        file.set_len(0)
            .map_err(|err| format!("truncate tf log segment {} failed: {err}", path.display()))?;
    } else {
        file.set_len(offset as u64)
            .map_err(|err| format!("trim tf log segment {} failed: {err}", path.display()))?;
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|err| format!("seek tf log segment {} failed: {err}", path.display()))?;
    }
    file.write_all(encoded)
        .map_err(|err| format!("write tf log segment {} failed: {err}", path.display()))?;
    file.flush()
        .map_err(|err| format!("flush tf log segment {} failed: {err}", path.display()))?;
    file.sync_all()
        .map_err(|err| format!("sync tf log segment {} failed: {err}", path.display()))?;
    Ok(())
}

fn load_tf_blocks() -> Result<Vec<TfLogBlock>, String> {
    let root = tf_log_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut blocks = Vec::new();
    for segment_index in 0..TF_LOG_SEGMENT_COUNT {
        let path = tf_segment_path(segment_index);
        let data = match read_tf_segment_file(&path) {
            Ok(Some(data)) => data,
            Ok(None) => continue,
            Err(err) => {
                println!(
                    "photoframe-rs: tf segment unreadable, reset {} reason={}",
                    path.display(),
                    err
                );
                clear_tf_segment(&path)?;
                continue;
            }
        };
        if data.is_empty() {
            continue;
        }
        let mut offset = 0usize;
        while offset + TF_LOG_BLOCK_HEADER_BYTES <= data.len() {
            let Some(block) = decode_tf_block(&data[offset..], segment_index, offset) else {
                break;
            };
            offset = block.end_offset;
            blocks.push(block);
        }
    }
    blocks.sort_by_key(|block| block.generation);
    Ok(blocks)
}

fn append_tf_snapshot(snapshot: &RestoredLogSnapshot) -> Result<(), String> {
    if snapshot.lines.is_empty() {
        return Ok(());
    }

    let encoded = encode_tf_block(snapshot, 0, current_epoch());
    if encoded.len() > TF_LOG_SEGMENT_CAP_BYTES {
        return Err(format!(
            "tf log block too large: {} > {}",
            encoded.len(),
            TF_LOG_SEGMENT_CAP_BYTES
        ));
    }

    if !tf_log_dir().exists() {
        return Err(format!("tf mount path missing: {}", tf_log_dir().display()));
    }
    let existing = load_tf_blocks()?;
    let last = existing.last();
    let next_generation = last.map(|block| block.generation + 1).unwrap_or(1);
    let encoded = encode_tf_block(snapshot, next_generation, current_epoch());

    let (segment_index, offset) = match last {
        Some(block) if block.end_offset + encoded.len() <= TF_LOG_SEGMENT_CAP_BYTES => {
            (block.segment_index, block.end_offset)
        }
        Some(block) => ((block.segment_index + 1) % TF_LOG_SEGMENT_COUNT, 0usize),
        None => (0usize, 0usize),
    };

    let path = tf_segment_path(segment_index);
    write_tf_segment_file(&path, offset, &encoded)?;
    let verify_blocks = load_tf_blocks()?;
    let Some(last_block) = verify_blocks.last() else {
        return Err("tf log verify failed: no block after write".into());
    };
    if last_block.generation != next_generation || last_block.line_count != snapshot.line_count {
        return Err(format!(
            "tf log verify mismatch: generation={} lines={} expected_generation={} expected_lines={}",
            last_block.generation,
            last_block.line_count,
            next_generation,
            snapshot.line_count
        ));
    }
    Ok(())
}

fn decode_persisted_log_snapshot() -> Option<RestoredLogSnapshot> {
    let snapshot = unsafe { PERSISTED_LOG_SNAPSHOT };
    if snapshot.magic != PERSISTED_LOG_MAGIC || snapshot.version != PERSISTED_LOG_VERSION {
        return None;
    }
    let used_bytes = snapshot.used_bytes as usize;
    if used_bytes == 0 || used_bytes > PERSISTED_LOG_MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(&snapshot.bytes[..used_bytes]).ok()?;
    let lines = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    Some(RestoredLogSnapshot {
        boot_id: snapshot.boot_id,
        line_count: snapshot.line_count as usize,
        truncated: snapshot.truncated != 0,
        lines,
    })
}

fn encode_persisted_log_snapshot(snapshot: &RestoredLogSnapshot) {
    let joined = snapshot.lines.join("\n");
    let bytes = joined.as_bytes();
    let used_bytes = bytes.len().min(PERSISTED_LOG_MAX_BYTES);

    unsafe {
        PERSISTED_LOG_SNAPSHOT.magic = PERSISTED_LOG_MAGIC;
        PERSISTED_LOG_SNAPSHOT.version = PERSISTED_LOG_VERSION;
        PERSISTED_LOG_SNAPSHOT.boot_id = snapshot.boot_id;
        PERSISTED_LOG_SNAPSHOT.used_bytes = used_bytes as u32;
        PERSISTED_LOG_SNAPSHOT.line_count = snapshot.line_count as u32;
        PERSISTED_LOG_SNAPSHOT.truncated = u32::from(snapshot.truncated);
        let dst = core::ptr::addr_of_mut!(PERSISTED_LOG_SNAPSHOT.bytes) as *mut u8;
        core::ptr::write_bytes(dst, 0, PERSISTED_LOG_MAX_BYTES);
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, used_bytes);
    }
}

fn clear_persisted_log_snapshot() {
    unsafe {
        PERSISTED_LOG_SNAPSHOT = PersistedLogSnapshot::empty();
    }
}

#[cfg(test)]
fn clear_persisted_log_snapshot_for_tests() {
    clear_persisted_log_snapshot();
}

fn global_log_buffer() -> &'static Mutex<LogBuffer> {
    static LOG_BUFFER: OnceLock<Mutex<LogBuffer>> = OnceLock::new();
    LOG_BUFFER.get_or_init(|| Mutex::new(LogBuffer::default()))
}

pub(crate) fn begin_boot_session(sd_history_ready: bool) {
    let history_blocks = if sd_history_ready {
        load_tf_blocks().ok()
    } else {
        None
    };
    let history_next_boot_id = if let Some(blocks) = history_blocks.as_ref() {
        blocks
            .last()
            .map(|block| block.boot_id.wrapping_add(1))
            .unwrap_or(1)
    } else {
        1
    };
    let restored = if sd_history_ready {
        clear_persisted_log_snapshot();
        None
    } else {
        decode_persisted_log_snapshot()
    };
    if let Ok(mut guard) = global_log_buffer().lock() {
        let next_boot_id = restored
            .as_ref()
            .map(|snapshot| snapshot.boot_id.wrapping_add(1))
            .unwrap_or(history_next_boot_id);
        guard.start_boot(next_boot_id, restored.as_ref());
    }
    if let Some(snapshot) = restored {
        append(
            "INFO",
            &format!(
                "photoframe-rs: restored rtc logs boot={} lines={} truncated={}",
                snapshot.boot_id,
                snapshot.line_count,
                i32::from(snapshot.truncated)
            ),
        );
    }
    if let Some(blocks) = history_blocks.as_ref() {
        let total_lines = blocks.iter().map(|block| block.line_count).sum::<usize>();
        let last_boot_id = blocks.last().map(|block| block.boot_id).unwrap_or(0);
        append(
            "INFO",
            &format!(
                "photoframe-rs: tf history ready blocks={} total_lines={} last_boot_id={}",
                blocks.len(), total_lines, last_boot_id
            ),
        );
    }
}

pub(crate) fn append(level: &str, message: &str) {
    if let Ok(mut guard) = global_log_buffer().lock() {
        guard.push(level, message);
    }
}

pub(crate) fn append_external(level: &str, message: &str) {
    append(level, message);
}

pub(crate) struct DeviceLogUploadCollector;

impl LogUploadProvider for DeviceLogUploadCollector {
    fn collect_logs(
        &mut self,
        config: &DeviceRuntimeConfig,
        request: &DeviceLogUploadRequest,
        uploaded_epoch: i64,
    ) -> Option<DeviceLogUploadRequestBody> {
        let history_blocks = if crate::sdcard::is_ready() {
            load_tf_blocks().unwrap_or_default()
        } else {
            Vec::new()
        };
        let history_total_lines = history_blocks.iter().map(|block| block.line_count).sum::<usize>();
        if crate::sdcard::is_ready() {
            append(
                "INFO",
                &format!(
                    "photoframe-rs: preparing log upload history_blocks={} history_lines={}",
                    history_blocks.len(),
                    history_total_lines
                ),
            );
        }

        let guard = global_log_buffer().lock().ok()?;
        let current_lines = guard.all_lines();
        let current_total_lines = current_lines.len();
        let current_total_bytes = guard.total_bytes;
        let current_boot_id = guard.boot_id;
        drop(guard);

        let max_lines = usize::try_from(request.max_lines).unwrap_or(LOG_BUFFER_MAX_LINES);
        let max_bytes = usize::try_from(request.max_bytes).unwrap_or(LOG_BUFFER_MAX_BYTES);
        let available_total_lines = current_total_lines
            + history_total_lines;
        let available_total_bytes = current_total_bytes
            + history_blocks.iter().map(|block| block.text_bytes).sum::<usize>();

        let mut selected: Vec<String> = Vec::new();
        let mut selected_bytes = 0usize;

        push_recent_lines(
            &mut selected,
            &mut selected_bytes,
            &current_lines,
            max_lines,
            max_bytes,
        );
        if selected.len() < max_lines && selected_bytes < max_bytes {
            for block in history_blocks.iter().rev() {
                let before_lines = selected.len();
                let before_bytes = selected_bytes;
                push_recent_lines(
                    &mut selected,
                    &mut selected_bytes,
                    &block.lines,
                    max_lines,
                    max_bytes,
                );
                if selected.len() == before_lines && selected_bytes == before_bytes {
                    break;
                }
                if selected.len() >= max_lines || selected_bytes >= max_bytes {
                    break;
                }
            }
        }
        selected.reverse();

        Some(DeviceLogUploadRequestBody {
            device_id: config.device_id.to_string(),
            request_id: request.request_id,
            uploaded_epoch,
            line_count: selected.len() as u32,
            truncated: selected.len() < available_total_lines,
            uploaded_bytes: Some(selected_bytes as u32),
            buffer_total_lines: Some(available_total_lines.min(u32::MAX as usize) as u32),
            buffer_total_bytes: Some(available_total_bytes.min(u32::MAX as usize) as u32),
            buffer_boot_id: Some(current_boot_id),
            lines: selected,
        })
    }
}

pub(crate) fn persist_for_next_boot() {
    let mut tf_result = Ok(());
    if let Ok(guard) = global_log_buffer().lock() {
        if crate::sdcard::is_ready() {
            if let Some(snapshot) = guard.snapshot_for_tf() {
                println!(
                    "photoframe-rs: tf persist snapshot boot={} lines={} truncated={}",
                    snapshot.boot_id,
                    snapshot.line_count,
                    i32::from(snapshot.truncated)
                );
                tf_result = append_tf_snapshot(&snapshot);
            }
        } else {
            let snapshot = guard.snapshot_for_rtc();
            encode_persisted_log_snapshot(&snapshot);
        }
    }

    if crate::sdcard::is_ready() {
        match tf_result {
            Ok(()) => {
                println!("photoframe-rs: tf persist success");
                thread::sleep(std::time::Duration::from_millis(100));
                clear_persisted_log_snapshot();
            }
            Err(err) => {
                if let Ok(guard) = global_log_buffer().lock() {
                    let snapshot = guard.snapshot_for_rtc();
                    encode_persisted_log_snapshot(&snapshot);
                }
                println!("photoframe-rs: tf persist failed: {err}");
                thread::sleep(std::time::Duration::from_millis(100));
                append(
                    "WARN",
                    &format!("photoframe-rs: tf log persist failed, fallback to rtc snapshot: {err}"),
                );
            }
        }
    }
}

#[macro_export]
macro_rules! device_log {
    ($level:expr, $($arg:tt)*) => {{
        let rendered = format!($($arg)*);
        println!("{}", rendered);
        $crate::diag::append($level, &rendered);
    }};
}

#[cfg(test)]
mod tests {
    use super::{
        DeviceLogUploadCollector, clear_persisted_log_snapshot_for_tests, global_log_buffer,
        load_tf_blocks, persist_for_next_boot, tf_log_dir, tf_segment_path,
        TF_LOG_SEGMENT_CAP_BYTES,
    };
    use photoframe_app::{DeviceRuntimeConfig, LogUploadProvider};
    use photoframe_contracts::DeviceLogUploadRequest;
    use std::sync::{Mutex, OnceLock};

    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn lock_test_guard() -> std::sync::MutexGuard<'static, ()> {
        test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reset_state_for_test() {
        clear_persisted_log_snapshot_for_tests();
        crate::sdcard::set_test_ready(false);
        if let Ok(mut guard) = global_log_buffer().lock() {
            guard.boot_id = 0;
            guard.next_seq = 0;
            guard.total_bytes = 0;
            guard.restored_prefix_lines = 0;
            guard.current_boot_truncated = false;
            guard.lines.clear();
        }
    }

    #[test]
    fn persisted_logs_survive_next_boot_session_without_sd_history() {
        let _test_guard = lock_test_guard();
        reset_state_for_test();

        super::begin_boot_session(false);
        super::append("INFO", "cycle one line a");
        super::append("WARN", "cycle one line b");
        persist_for_next_boot();

        super::begin_boot_session(false);
        super::append("INFO", "cycle two line c");

        let mut collector = DeviceLogUploadCollector;
        let payload = collector
            .collect_logs(
                &DeviceRuntimeConfig {
                    device_id: "pf-test".into(),
                    ..DeviceRuntimeConfig::default()
                },
                &DeviceLogUploadRequest {
                    request_id: 1,
                    max_lines: 20,
                    max_bytes: 4096,
                    reason: None,
                    created_epoch: 0,
                    expires_epoch: None,
                },
                123,
            )
            .expect("payload");

        assert_eq!(payload.buffer_boot_id, Some(2));
        let joined = payload.lines.join("\n");
        assert!(joined.contains("cycle one line a"));
        assert!(joined.contains("cycle one line b"));
        assert!(joined.contains("restored rtc logs"));
        assert!(joined.contains("cycle two line c"));
    }

    #[test]
    fn tf_blocks_are_capped_and_read_back() {
        let _test_guard = lock_test_guard();
        reset_state_for_test();

        let temp_dir = std::env::temp_dir().join(format!("photoframe-diag-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        unsafe {
            std::env::set_var("PHOTOFRAME_TF_LOG_DIR", &temp_dir);
        }
        crate::sdcard::set_test_ready(true);

        super::begin_boot_session(true);
        super::append("INFO", "boot a line 1");
        super::append("INFO", "boot a line 2");
        persist_for_next_boot();

        super::begin_boot_session(true);
        super::append("INFO", "boot b line 3");
        persist_for_next_boot();

        let blocks = load_tf_blocks().expect("load tf blocks");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].lines.join("\n").contains("boot a line 1"));
        assert!(blocks[1].lines.join("\n").contains("boot b line 3"));

        let cleanup_dir = tf_log_dir();
        crate::sdcard::set_test_ready(false);
        unsafe {
            std::env::remove_var("PHOTOFRAME_TF_LOG_DIR");
        }
        let _ = std::fs::remove_dir_all(cleanup_dir);
    }

    #[test]
    fn unreadable_tf_segment_is_reset_before_new_snapshot_write() {
        let _test_guard = lock_test_guard();
        reset_state_for_test();

        let temp_dir = std::env::temp_dir().join(format!("photoframe-diag-reset-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        unsafe {
            std::env::set_var("PHOTOFRAME_TF_LOG_DIR", &temp_dir);
        }
        crate::sdcard::set_test_ready(true);

        std::fs::write(tf_segment_path(0), vec![0u8; TF_LOG_SEGMENT_CAP_BYTES + 1])
            .expect("seed oversized segment");

        super::begin_boot_session(true);
        super::append("INFO", "boot after corrupted tf segment");
        persist_for_next_boot();

        let blocks = load_tf_blocks().expect("load tf blocks");
        assert_eq!(blocks.len(), 1);
        assert!(
            blocks[0]
                .lines
                .join("\n")
                .contains("boot after corrupted tf segment")
        );

        let cleanup_dir = tf_log_dir();
        crate::sdcard::set_test_ready(false);
        unsafe {
            std::env::remove_var("PHOTOFRAME_TF_LOG_DIR");
        }
        let _ = std::fs::remove_dir_all(cleanup_dir);
    }
}
