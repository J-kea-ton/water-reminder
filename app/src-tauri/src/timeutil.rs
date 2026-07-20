use chrono::{Duration, Local, TimeZone, Timelike};

pub fn now_epoch() -> i64 {
    Local::now().timestamp()
}

pub fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

// 本地日期往前推 days 天的日期串。date_before(0) == today_str()。
// 用于补齐统计图的连续日期轴。
pub fn date_before(days: i64) -> String {
    (Local::now().date_naive() - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

pub fn now_minutes() -> u32 {
    let n = Local::now();
    n.hour() * 60 + n.minute()
}

pub fn parse_hhmm(s: &str) -> u32 {
    let mut p = s.split(':');
    let h: u32 = p.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    let m: u32 = p.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    (h.min(23)) * 60 + m.min(59)
}

// 分钟数是否落在 [start, end) 内，支持跨午夜（end <= start 视为跨天）。
pub fn in_range(now: u32, start: u32, end: u32) -> bool {
    if start == end {
        false
    } else if start < end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

pub fn epoch_to_hhmm(e: i64) -> String {
    Local
        .timestamp_opt(e, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".into())
}
