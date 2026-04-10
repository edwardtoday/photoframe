use chrono::{Days, NaiveDate};

pub fn split_url_origin_and_rest(url: &str) -> Option<(String, String)> {
    let scheme_pos = url.find("://")?;
    let host_start = scheme_pos + 3;
    if let Some(path_pos) = url[host_start..].find('/') {
        let absolute = host_start + path_pos;
        Some((url[..absolute].to_string(), url[absolute..].to_string()))
    } else {
        Some((url.to_string(), "/".to_string()))
    }
}

pub fn normalize_origin(origin: &str) -> String {
    let mut out = origin.to_string();
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

pub fn build_url_with_origin(url: &str, origin: &str) -> Option<String> {
    let (_url_origin, url_rest) = split_url_origin_and_rest(url)?;
    let normalized = normalize_origin(origin);
    let (origin_part, origin_rest) = split_url_origin_and_rest(&normalized)?;
    if origin_rest != "/" {
        return None;
    }
    Some(format!("{origin_part}{url_rest}"))
}

pub fn shift_date_param_days(url: &str, delta_days: i64) -> Option<String> {
    if delta_days == 0 {
        return None;
    }
    let marker = "date=";
    let marker_pos = url.find(marker)?;
    let date_pos = marker_pos + marker.len();
    let date_text = url.get(date_pos..date_pos + 10)?;
    let date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").ok()?;
    let shifted = if delta_days > 0 {
        date.checked_add_days(Days::new(delta_days as u64))?
    } else {
        date.checked_sub_days(Days::new((-delta_days) as u64))?
    };
    let mut out = url.to_string();
    out.replace_range(
        date_pos..date_pos + 10,
        &shifted.format("%Y-%m-%d").to_string(),
    );
    Some(out)
}

pub fn shift_date_string_days(date_text: &str, delta_days: i64) -> Option<String> {
    if delta_days == 0 {
        return Some(date_text.to_string());
    }
    let date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").ok()?;
    let shifted = if delta_days > 0 {
        date.checked_add_days(Days::new(delta_days as u64))?
    } else {
        date.checked_sub_days(Days::new((-delta_days) as u64))?
    };
    Some(shifted.format("%Y-%m-%d").to_string())
}

pub fn date_days_behind(reference_date: &str, candidate_date: &str) -> Option<i64> {
    let reference = NaiveDate::parse_from_str(reference_date, "%Y-%m-%d").ok()?;
    let candidate = NaiveDate::parse_from_str(candidate_date, "%Y-%m-%d").ok()?;
    Some((reference - candidate).num_days())
}

pub fn extract_date_from_url(url: &str) -> Option<String> {
    if let Some(marker_pos) = url.find("date=") {
        let start = marker_pos + "date=".len();
        let candidate = url.get(start..start + 10)?;
        if NaiveDate::parse_from_str(candidate, "%Y-%m-%d").is_ok() {
            return Some(candidate.to_string());
        }
    }

    if let Some(marker_pos) = url.find("daily-") {
        let start = marker_pos + "daily-".len();
        let candidate = url.get(start..start + 10)?;
        if NaiveDate::parse_from_str(candidate, "%Y-%m-%d").is_ok() {
            return Some(candidate.to_string());
        }
    }

    None
}

pub fn build_dated_url(template: &str, date: &str, device_id: &str) -> String {
    let safe_device_id = if device_id.is_empty() {
        "unknown"
    } else {
        device_id
    };
    let mut url = template
        .replace("%DATE%", date)
        .replace("%DEVICE_ID%", safe_device_id);

    if !device_id.is_empty() && !url.contains("device_id=") {
        let fragment_pos = url.find('#');
        let (mut base, fragment) = match fragment_pos {
            Some(pos) => (url[..pos].to_string(), url[pos..].to_string()),
            None => (url.clone(), String::new()),
        };
        let connector = if base.contains('?') { '&' } else { '?' };
        base.push(connector);
        base.push_str("device_id=");
        base.push_str(&url_encode_component(device_id));
        url = format!("{base}{fragment}");
    }

    url
}

pub fn build_fetch_url_candidates(primary_url: &str, preferred_origin: &str) -> Vec<String> {
    let mut base_urls = Vec::new();
    if !preferred_origin.is_empty() {
        if let Some(preferred_url) = build_url_with_origin(primary_url, preferred_origin) {
            add_unique_url(preferred_url, &mut base_urls);
        }
    }
    add_unique_url(primary_url.to_string(), &mut base_urls);

    let mut candidates = Vec::new();
    for url in &base_urls {
        add_unique_url(url.clone(), &mut candidates);
    }
    for url in &base_urls {
        if let Some(fallback) = shift_date_param_days(url, -1) {
            add_unique_url(fallback, &mut candidates);
        }
    }
    candidates
}

pub fn build_checkin_base_url_candidates(
    orchestrator_base_url: &str,
    fetch_url_used: &str,
    fallback_url: &str,
    preferred_image_origin: &str,
    image_url_template: &str,
) -> Vec<String> {
    let mut candidates = Vec::new();
    add_unique_url(normalize_origin(orchestrator_base_url), &mut candidates);

    for value in [
        fetch_url_used,
        preferred_image_origin,
        fallback_url,
        image_url_template,
    ] {
        if let Some((origin, _)) = split_url_origin_and_rest(value) {
            add_unique_url(origin, &mut candidates);
        }
    }

    candidates
}

fn add_unique_url(url: String, urls: &mut Vec<String>) {
    if url.is_empty() || urls.iter().any(|item| item == &url) {
        return;
    }
    urls.push(url);
}

fn url_encode_component(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}
