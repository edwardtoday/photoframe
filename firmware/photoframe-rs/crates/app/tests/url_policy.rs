use photoframe_app::{
    build_checkin_base_url_candidates, build_dated_url, build_fetch_url_candidates,
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
