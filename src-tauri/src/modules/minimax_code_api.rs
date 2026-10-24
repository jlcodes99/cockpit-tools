/// Best-effort IANA timezone name for the `timezone_id` query parameter.
/// MiniMax rejects requests without it (`1406010011 invalid timezone_id`).
pub fn iana_timezone() -> String {
    iana_time_zone::get_timezone()
        .unwrap_or_else(|_| "Asia/Shanghai".to_string())
}
