use photoframe_app::{
    build_checkin_base_url_candidates, build_dated_url, build_fetch_url_candidates,
    date_days_behind, extract_date_from_url, shift_date_string_days,
};

#[test]
fn build_dated_url_replaces_placeholders_and_appends_device_id() {
    let url = build_dated_url(
        "https://example.com/daily.bmp?date=%DATE%",
        "2026-03-07",
        "pf livingroom",
    );

    assert_eq!(
        url,
        "https://example.com/daily.bmp?date=2026-03-07&device_id=pf%20livingroom"
    );
}

#[test]
fn build_fetch_url_candidates_prefers_previous_origin_and_yesterday() {
    let candidates = build_fetch_url_candidates(
        "https://public.example.com/daily.bmp?date=2026-03-07",
        "http://192.168.1.10:18081",
    );

    assert_eq!(
        candidates,
        vec![
            "http://192.168.1.10:18081/daily.bmp?date=2026-03-07",
            "https://public.example.com/daily.bmp?date=2026-03-07",
            "http://192.168.1.10:18081/daily.bmp?date=2026-03-06",
            "https://public.example.com/daily.bmp?date=2026-03-06",
        ]
    );
}

#[test]
fn build_checkin_candidates_are_unique_and_include_fetch_and_fallback_origins() {
    let candidates = build_checkin_base_url_candidates(
        "http://192.168.1.10:18081",
        "https://public.example.com/assets/1.jpg",
        "http://192.168.1.10:18081/public/daily.bmp",
        "https://public.example.com",
        "https://public.example.com/daily.bmp",
    );

    assert_eq!(
        candidates,
        vec!["http://192.168.1.10:18081", "https://public.example.com"]
    );
}

#[test]
fn extract_date_from_url_supports_query_and_asset_name() {
    assert_eq!(
        extract_date_from_url("https://example.com/public/daily.bmp?date=2026-04-10"),
        Some("2026-04-10".into())
    );
    assert_eq!(
        extract_date_from_url(
            "https://example.com/api/v1/assets/daily-2026-04-09-sierra-reference.bmp"
        ),
        Some("2026-04-09".into())
    );
}

#[test]
fn shift_date_string_and_distance_work_for_history_browse() {
    assert_eq!(
        shift_date_string_days("2026-04-10", -3),
        Some("2026-04-07".into())
    );
    assert_eq!(date_days_behind("2026-04-10", "2026-04-07"), Some(3));
}
