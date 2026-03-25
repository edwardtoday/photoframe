#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use photoframe_app::{DeviceRuntimeConfig, LogUploadProvider};
use photoframe_contracts::{DeviceLogUploadRequest, DeviceLogUploadRequestBody};

const LOG_BUFFER_MAX_BYTES: usize = 12 * 1024;
const LOG_BUFFER_MAX_LINES: usize = 200;
const LOG_LINE_MAX_CHARS: usize = 220;

#[derive(Default)]
struct LogBuffer {
    boot_id: u32,
    next_seq: u64,
    total_bytes: usize,
    lines: VecDeque<String>,
}

impl LogBuffer {
    fn reset_for_boot(&mut self) {
        self.boot_id = self.boot_id.wrapping_add(1);
        self.next_seq = 0;
        self.total_bytes = 0;
        self.lines.clear();
    }

    fn push(&mut self, level: &str, message: &str) {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        self.next_seq = self.next_seq.wrapping_add(1);
        let body: String = message.chars().take(LOG_LINE_MAX_CHARS).collect();
        let line = format!(
            "[{}][boot:{}][seq:{}][{}] {}",
            epoch, self.boot_id, self.next_seq, level, body
        );
        self.total_bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > LOG_BUFFER_MAX_LINES || self.total_bytes > LOG_BUFFER_MAX_BYTES {
            if let Some(removed) = self.lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(removed.len());
            } else {
                self.total_bytes = 0;
                break;
            }
        }
    }

    fn snapshot(
        &self,
        device_id: &str,
        request: &DeviceLogUploadRequest,
        uploaded_epoch: i64,
    ) -> DeviceLogUploadRequestBody {
        let max_lines = usize::try_from(request.max_lines).unwrap_or(LOG_BUFFER_MAX_LINES);
        let max_bytes = usize::try_from(request.max_bytes).unwrap_or(LOG_BUFFER_MAX_BYTES);
        let mut selected: Vec<String> = Vec::new();
        let mut selected_bytes = 0usize;

        for line in self.lines.iter().rev() {
            let line_bytes = line.len();
            if selected.len() >= max_lines {
                break;
            }
            if !selected.is_empty() && selected_bytes + line_bytes > max_bytes {
                break;
            }
            if selected.is_empty() && line_bytes > max_bytes {
                let clipped: String = line.chars().take(LOG_LINE_MAX_CHARS.min(max_bytes)).collect();
                selected.push(clipped);
                break;
            }
            selected.push(line.clone());
            selected_bytes += line_bytes;
        }
        selected.reverse();

        DeviceLogUploadRequestBody {
            device_id: device_id.to_string(),
            request_id: request.request_id,
            uploaded_epoch,
            line_count: selected.len() as u32,
            truncated: selected.len() < self.lines.len(),
            lines: selected,
        }
    }
}

fn global_log_buffer() -> &'static Mutex<LogBuffer> {
    static LOG_BUFFER: OnceLock<Mutex<LogBuffer>> = OnceLock::new();
    LOG_BUFFER.get_or_init(|| Mutex::new(LogBuffer::default()))
}

pub(crate) fn begin_boot_session() {
    if let Ok(mut guard) = global_log_buffer().lock() {
        guard.reset_for_boot();
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
        let guard = global_log_buffer().lock().ok()?;
        Some(guard.snapshot(&config.device_id, request, uploaded_epoch))
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
