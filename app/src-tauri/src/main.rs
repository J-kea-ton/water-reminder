#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod model;
mod screenscan;
mod timeutil;

use model::{AppState, Config, DayStat, DrinkEntry, Persisted};
use timeutil::*;

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    interval_minutes: u32,
    active_start: String,
    active_end: String,
    cup_ml: u32,
    daily_goal_ml: u32,
    goal_cups: u32,
    paused: bool,
    dnd_enabled: bool,
    dnd_start: String,
    dnd_end: String,
    sound_enabled: bool,
    onboarding_shown: bool,
    count: u32,
    total_ml: u32,
    next_label: String,
    pet_state: String,
    sleep_reason: String,
    sleep_recover_time: String, // 睡觉时给用户看的恢复时间
    day_just_reset: bool,       // 首次拉取且今天还没喝→用于主窗展示「新的一天」
}

fn ensure_today(p: &mut Persisted) -> bool {
    let t = today_str();
    if p.day.date != t {
        archive_day(p); // 清空前先把要结束的那天存进 history
        p.day.date = t;
        p.day.count = 0;
        p.day.total_ml = 0;
        p.day.last_drink_epoch = 0;
        p.day.drink_log.clear();
        true
    } else {
        false
    }
}

// 把 p.day（要结束的那天）归档到 history。
// 只归档真喝过水的天；按 date 判重保证幂等（一天内多次调 ensure_today 不重复 push）；裁剪到最近 30 条。
fn archive_day(p: &mut Persisted) {
    if p.day.date.is_empty() || p.day.total_ml == 0 {
        return;
    }
    if p.history.iter().any(|d| d.date == p.day.date) {
        return;
    }
    p.history.push(DayStat {
        date: p.day.date.clone(),
        total_ml: p.day.total_ml,
        goal_ml: p.config.daily_goal_ml, // 用当天的目标，不是今天的
    });
    let len = p.history.len();
    if len > 30 {
        p.history.drain(0..len - 30);
    }
}

// 老版本数据没有 total_ml / drink_log，启动时按 cup_ml 补出近似值。
fn migrate_legacy(p: &mut Persisted) {
    if p.day.total_ml == 0 && p.day.count > 0 {
        p.day.total_ml = p.day.count * p.config.cup_ml;
    }
    if p.day.drink_log.is_empty() && p.day.count > 0 {
        // 把已有的 total_ml 按杯数摊回去，余数并进最后一条，
        // 保证各条之和恰好等于 total_ml——否则撤销会扣出错误的数字。
        let n = p.day.count;
        let each = p.day.total_ml / n;
        let rest = p.day.total_ml % n;
        let ep = p.day.last_drink_epoch;
        for i in 0..n {
            let ml = if i == n - 1 { each + rest } else { each };
            p.day.drink_log.push(DrinkEntry { ml, epoch: ep });
        }
    }
}

fn goal_cups(c: &Config) -> u32 {
    if c.cup_ml == 0 {
        return 1;
    }
    (c.daily_goal_ml + c.cup_ml - 1) / c.cup_ml
}

// 优先级：暂停 → 已达标（goal 醒着可点） → 免打扰 → 下班 → 待机。
// sleep_reason 只在 pet 真的睡觉时非空；goal 态不算睡觉。
fn derive_state(p: &Persisted) -> (String, String) {
    if p.config.paused {
        return ("sleep".into(), "pause".into());
    }
    if p.day.total_ml >= p.config.daily_goal_ml {
        return ("goal".into(), String::new());
    }
    let nm = now_minutes();
    if p.config.dnd_enabled
        && in_range(nm, parse_hhmm(&p.config.dnd_start), parse_hhmm(&p.config.dnd_end))
    {
        return ("sleep".into(), "dnd".into());
    }
    if !in_range(
        nm,
        parse_hhmm(&p.config.active_start),
        parse_hhmm(&p.config.active_end),
    ) {
        return ("sleep".into(), "offhours".into());
    }
    ("idle".into(), String::new())
}

fn next_label(p: &Persisted) -> String {
    let c = &p.config;
    if c.paused {
        return "已暂停".into();
    }
    if p.day.total_ml >= c.daily_goal_ml {
        return "今天喝够啦".into();
    }
    let nm = now_minutes();
    if c.dnd_enabled && in_range(nm, parse_hhmm(&c.dnd_start), parse_hhmm(&c.dnd_end)) {
        return format!("午休中，{} 恢复", c.dnd_end);
    }
    if !in_range(nm, parse_hhmm(&c.active_start), parse_hhmm(&c.active_end)) {
        return format!("明早 {} 见", c.active_start);
    }
    let base = if p.day.last_drink_epoch > 0 {
        p.day.last_drink_epoch
    } else {
        now_epoch()
    };
    epoch_to_hhmm(base + (c.interval_minutes as i64) * 60)
}

fn sleep_recover(p: &Persisted, reason: &str) -> String {
    match reason {
        "pause" => "点菜单栏可以继续".into(),
        "dnd" => format!("{} 后恢复", p.config.dnd_end),
        "offhours" => format!("明早 {} 见", p.config.active_start),
        _ => String::new(),
    }
}

fn build_snapshot(p: &Persisted, day_just_reset: bool) -> Snapshot {
    let (pet, reason) = derive_state(p);
    let recover = sleep_recover(p, &reason);
    let gc = goal_cups(&p.config);
    Snapshot {
        interval_minutes: p.config.interval_minutes,
        active_start: p.config.active_start.clone(),
        active_end: p.config.active_end.clone(),
        cup_ml: p.config.cup_ml,
        daily_goal_ml: p.config.daily_goal_ml,
        goal_cups: gc,
        paused: p.config.paused,
        dnd_enabled: p.config.dnd_enabled,
        dnd_start: p.config.dnd_start.clone(),
        dnd_end: p.config.dnd_end.clone(),
        sound_enabled: p.config.sound_enabled,
        onboarding_shown: p.config.onboarding_shown,
        count: p.day.count,
        total_ml: p.day.total_ml,
        next_label: next_label(p),
        pet_state: pet,
        sleep_reason: reason,
        sleep_recover_time: recover,
        day_just_reset,
    }
}

// 基本条件已满足（活跃时段/未暂停/未达标/距上次喝水够久）后，
// 结合「待会儿」的稍后时刻和上次提醒时刻，判断此刻是否该弹提醒。
// 稍后中：到点(now>=snooze_until)才提醒；否则按常规间隔。
fn remind_due(base_ok: bool, now: i64, last_notify: i64, snooze_until: i64, interval: i64) -> bool {
    if !base_ok {
        return false;
    }
    if snooze_until > 0 {
        now >= snooze_until
    } else {
        now - last_notify >= interval
    }
}

fn should_remind(p: &Persisted) -> bool {
    if p.config.paused || p.day.total_ml >= p.config.daily_goal_ml {
        return false;
    }
    let nm = now_minutes();
    if !in_range(
        nm,
        parse_hhmm(&p.config.active_start),
        parse_hhmm(&p.config.active_end),
    ) {
        return false;
    }
    if p.config.dnd_enabled
        && in_range(nm, parse_hhmm(&p.config.dnd_start), parse_hhmm(&p.config.dnd_end))
    {
        return false;
    }
    let interval = (p.config.interval_minutes as i64) * 60;
    let base = if p.day.last_drink_epoch > 0 {
        p.day.last_drink_epoch
    } else {
        now_epoch() - interval
    };
    now_epoch() - base >= interval
}

fn clamp_config(c: &mut Config) {
    if c.interval_minutes < 15 {
        c.interval_minutes = 15;
    }
    if c.interval_minutes > 180 {
        c.interval_minutes = 180;
    }
    if c.cup_ml < 100 {
        c.cup_ml = 100;
    }
    if c.cup_ml > 1000 {
        c.cup_ml = 1000;
    }
    if c.daily_goal_ml < 500 {
        c.daily_goal_ml = 500;
    }
    if c.daily_goal_ml > 5000 {
        c.daily_goal_ml = 5000;
    }
}

fn persist(state: &State<AppState>, p: &Persisted) {
    let path = state.data_path.lock().unwrap().clone();
    p.save(&path);
}

fn emit_update(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snap = {
        let mut p = state.persisted.lock().unwrap();
        let reset = ensure_today(&mut p);
        build_snapshot(&p, reset)
    };
    let _ = app.emit("updated", snap);
}

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> Snapshot {
    let mut p = state.persisted.lock().unwrap();
    let reset = ensure_today(&mut p);
    build_snapshot(&p, reset)
}

// 统计页的一天：连续日期轴上的一个点。缺记录的天补 total_ml=0。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsPoint {
    date: String,
    total_ml: u32,
    goal_ml: u32,
    is_today: bool,
}

// 截止到今天（含）最近 n 天的日期，oldest→newest。
fn recent_dates(n: i64) -> Vec<String> {
    (0..n).rev().map(date_before).collect()
}

// 把 history + 今天的实时数据，按给定日期轴摊成连续的点。
// 达标率/连续/日均等统计放前端算，后端只负责给齐这 n 个点。
fn assemble_stats(p: &Persisted, dates: &[String]) -> Vec<StatsPoint> {
    dates
        .iter()
        .map(|dt| {
            if *dt == p.day.date {
                StatsPoint {
                    date: dt.clone(),
                    total_ml: p.day.total_ml,
                    goal_ml: p.config.daily_goal_ml,
                    is_today: true,
                }
            } else if let Some(h) = p.history.iter().find(|h| &h.date == dt) {
                StatsPoint {
                    date: dt.clone(),
                    total_ml: h.total_ml,
                    goal_ml: h.goal_ml,
                    is_today: false,
                }
            } else {
                // 没喝的天补 0，目标用当前配置（total 0 < goal，前端自然算作没达标）
                StatsPoint {
                    date: dt.clone(),
                    total_ml: 0,
                    goal_ml: p.config.daily_goal_ml,
                    is_today: false,
                }
            }
        })
        .collect()
}

#[tauri::command]
fn get_stats(state: State<AppState>) -> Vec<StatsPoint> {
    let mut p = state.persisted.lock().unwrap();
    if ensure_today(&mut p) {
        persist(&state, &p); // 打开统计页时若正好跨了天，把归档落盘
    }
    let dates = recent_dates(30);
    assemble_stats(&p, &dates)
}

// 记一杯：往 drink_log 追加一条，count 由 log 长度派生。
// 返回本次跨过的里程碑（goal / half），没跨过返回 None。
fn apply_drink(p: &mut Persisted, ml: Option<u32>, now: i64) -> Option<&'static str> {
    let actual_ml = ml.unwrap_or(p.config.cup_ml);
    let prev_total = p.day.total_ml;
    let goal_ml = p.config.daily_goal_ml;
    let half_ml = goal_ml / 2;
    p.day.drink_log.push(DrinkEntry {
        ml: actual_ml,
        epoch: now,
    });
    p.day.count = p.day.drink_log.len() as u32;
    p.day.total_ml += actual_ml;
    p.day.last_drink_epoch = now;
    if prev_total < goal_ml && p.day.total_ml >= goal_ml {
        Some("goal")
    } else if prev_total < half_ml && p.day.total_ml >= half_ml {
        Some("half")
    } else {
        None
    }
}

// 撤销最后一杯。log 为空时不动任何字段，返回 false。
fn apply_undo(p: &mut Persisted) -> bool {
    match p.day.drink_log.pop() {
        Some(entry) => {
            p.day.total_ml = p.day.total_ml.saturating_sub(entry.ml);
            p.day.count = p.day.drink_log.len() as u32;
            p.day.last_drink_epoch = p.day.drink_log.last().map(|e| e.epoch).unwrap_or(0);
            true
        }
        None => false,
    }
}

#[tauri::command]
fn drink(app: AppHandle, state: State<AppState>, ml: Option<u32>) -> Snapshot {
    let (snap, milestone) = {
        let mut p = state.persisted.lock().unwrap();
        ensure_today(&mut p);
        let m = apply_drink(&mut p, ml, now_epoch());
        persist(&state, &p);
        (build_snapshot(&p, false), m)
    };
    *state.snooze_until.lock().unwrap() = 0; // 喝了就取消稍后
    let _ = app.emit("updated", snap.clone());
    let _ = app.emit("pet-happy", ());
    if let Some(kind) = milestone {
        let _ = app.emit("pet-milestone", serde_json::json!({ "kind": kind }));
    }
    snap
}

// 撤销一杯（对应前端「刚才点错了」）
#[tauri::command]
fn undo_drink(app: AppHandle, state: State<AppState>) -> Snapshot {
    let snap = {
        let mut p = state.persisted.lock().unwrap();
        ensure_today(&mut p);
        if apply_undo(&mut p) {
            persist(&state, &p);
        }
        build_snapshot(&p, false)
    };
    let _ = app.emit("updated", snap.clone());
    snap
}

// 点「待会儿」：15 分钟后再提醒
#[tauri::command]
fn snooze(state: State<AppState>) {
    *state.snooze_until.lock().unwrap() = now_epoch() + 15 * 60;
}

#[tauri::command]
fn save_config(app: AppHandle, state: State<AppState>, config: Config) -> Snapshot {
    let snap = {
        let mut p = state.persisted.lock().unwrap();
        let onboarded = p.config.onboarding_shown;
        p.config = config;
        p.config.onboarding_shown = onboarded;
        clamp_config(&mut p.config);
        ensure_today(&mut p);
        persist(&state, &p);
        build_snapshot(&p, false)
    };
    let _ = app.emit("updated", snap.clone());
    snap
}

#[tauri::command]
fn toggle_pause(app: AppHandle, state: State<AppState>) -> Snapshot {
    let snap = {
        let mut p = state.persisted.lock().unwrap();
        p.config.paused = !p.config.paused;
        ensure_today(&mut p);
        persist(&state, &p);
        build_snapshot(&p, false)
    };
    let _ = app.emit("updated", snap.clone());
    snap
}

#[tauri::command]
fn show_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[tauri::command]
fn hide_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.hide();
    }
}

// 从 from 窗口切到 to 窗口：让 to 出现在 from 当前所在位置，再隐藏 from。
// 三个面板窗口尺寸相同，位置对齐即视觉上"原地替换"，不会跳回屏幕中央。
#[tauri::command]
fn switch_window(app: AppHandle, from: String, to: String) {
    let (fw, tw) = match (
        app.get_webview_window(&from),
        app.get_webview_window(&to),
    ) {
        (Some(f), Some(t)) => (f, t),
        _ => return,
    };
    if let Ok(pos) = fw.outer_position() {
        let _ = tw.set_position(pos); // 先对齐位置，再显示，避免先在中央闪一下
    }
    let _ = tw.show();
    let _ = tw.set_focus();
    let _ = fw.hide();
}

#[tauri::command]
fn toggle_pet_visible(app: AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
        }
    }
}

// 桌宠缩到屏幕右边缘，只露 44px，Y 固定在安全位置避免跑丢
#[tauri::command]
fn pet_retract(app: AppHandle, state: State<AppState>) {
    if let Some(win) = app.get_webview_window("pet") {
        if let Ok(Some(mon)) = win.primary_monitor() {
            let sz = mon.size();
            let mp = mon.position();
            let win_sz = win.outer_size().unwrap_or_default();
            let cur = win.outer_position().ok();
            let retract_x = mp.x + sz.width as i32 - 80;
            let is_retracted = cur.map(|p| p.x >= retract_x - 8).unwrap_or(false);
            // 只在首次缩起时保存当前位置，避免二次缩起把「跑丢」的坐标当原位置
            if !is_retracted && state.pet_saved_pos.lock().unwrap().is_none() {
                if let Some(p) = cur {
                    // 只保存位于可见屏幕内的位置，防止把屏幕外坐标当原位置
                    let vx = mp.x;
                    let vy = mp.y;
                    let vw = sz.width as i32;
                    let vh = sz.height as i32;
                    if p.x >= vx - 20
                        && p.x <= vx + vw - 40
                        && p.y >= vy - 20
                        && p.y <= vy + vh - 40
                    {
                        *state.pet_saved_pos.lock().unwrap() = Some((p.x, p.y));
                    }
                }
            }
            // Y 固定在屏幕中下部，永远可见
            let y = mp.y + sz.height as i32 - win_sz.height as i32 - 120;
            let _ = win.set_position(PhysicalPosition::new(retract_x, y));
        }
    }
}

// 从缩起状态展开回原位置；saved 缺失就落到安全默认（屏幕右下）
#[tauri::command]
fn pet_expand(app: AppHandle, state: State<AppState>) {
    if let Some(win) = app.get_webview_window("pet") {
        let saved = state.pet_saved_pos.lock().unwrap().take();
        if let Some((x, y)) = saved {
            let _ = win.set_position(PhysicalPosition::new(x, y));
        } else {
            reset_pet_position_impl(&win);
        }
    }
}

// 强制把桌宠拉回屏幕右下角。托盘菜单「水豚回家」也走这个。
fn reset_pet_position_impl(win: &tauri::WebviewWindow) {
    if let Ok(Some(mon)) = win.primary_monitor() {
        let sz = mon.size();
        let mp = mon.position();
        let win_sz = win.outer_size().unwrap_or_default();
        let x = mp.x + sz.width as i32 - win_sz.width as i32 - 24;
        let y = mp.y + sz.height as i32 - win_sz.height as i32 - 80;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
fn pet_go_home(app: AppHandle, state: State<AppState>) {
    *state.pet_saved_pos.lock().unwrap() = None;
    if let Some(win) = app.get_webview_window("pet") {
        let _ = win.show();
        reset_pet_position_impl(&win);
    }
}


// 前端拖动桌宠时记一下时刻，刚拖完几秒内不自动挪
#[tauri::command]
fn pet_touched(state: State<AppState>) {
    *state.last_drag_epoch.lock().unwrap() = now_epoch();
}

// 重置今天喝水杯数（方便测试彩虹里程碑）
#[tauri::command]
fn reset_today(app: AppHandle, state: State<AppState>) -> Snapshot {
    let snap = {
        let mut p = state.persisted.lock().unwrap();
        ensure_today(&mut p);
        p.day.count = 0;
        p.day.total_ml = 0;
        p.day.last_drink_epoch = 0;
        p.day.drink_log.clear();
        persist(&state, &p);
        build_snapshot(&p, false)
    };
    let _ = app.emit("updated", snap.clone());
    snap
}

#[tauri::command]
fn mark_onboarding(state: State<AppState>) {
    let mut p = state.persisted.lock().unwrap();
    p.config.onboarding_shown = true;
    persist(&state, &p);
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// 让桌宠能出现在别的 app 的全屏画面上。
//
// macOS 里 app 全屏不是把窗口拉大，而是新开一个独立的 Space。别的窗口想浮上去，
// 光有 alwaysOnTop + visibleOnAllWorkspaces 不够——tao 的 set_visible_on_all_workspaces
// 只设了 CanJoinAllSpaces，缺 FullScreenAuxiliary，桌宠会整个留在原来的桌面上不渲染。
// Tauri 没暴露这个标志，只能拿 ns_window 自己设。
#[cfg(target_os = "macos")]
fn allow_pet_over_fullscreen(app: &AppHandle) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    let win = match app.get_webview_window("pet") {
        Some(w) => w,
        None => return,
    };
    let ptr = match win.ns_window() {
        Ok(p) if !p.is_null() => p,
        _ => return,
    };
    unsafe {
        let ns: &NSWindow = &*(ptr as *const NSWindow);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary,
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn allow_pet_over_fullscreen(_app: &AppHandle) {}

// 打开 app 时把桌宠放到主面板右边、垂直居中对齐，用户一眼就能看到；之后可以自己拖走。
// 拿不到主面板位置时退回屏幕右侧中部。
fn position_pet_beside_panel(app: &AppHandle) {
    let pet = match app.get_webview_window("pet") {
        Some(w) => w,
        None => return,
    };
    let mon = match pet.primary_monitor() {
        Ok(Some(m)) => m,
        _ => return,
    };
    let sz = mon.size();
    let mp = mon.position();
    let pet_sz = pet.outer_size().unwrap_or_default();
    let gap = 20;

    // 优先贴着主面板右侧
    let beside = app
        .get_webview_window("main")
        .and_then(|m| m.outer_position().ok().zip(m.outer_size().ok()))
        .map(|(mpos, msz)| {
            let x = mpos.x + msz.width as i32 + gap;
            let y = mpos.y + (msz.height as i32 - pet_sz.height as i32) / 2;
            (x, y)
        });

    let (mut x, y) = beside.unwrap_or_else(|| {
        let x = mp.x + sz.width as i32 - pet_sz.width as i32 - 40;
        let y = mp.y + (sz.height as i32 - pet_sz.height as i32) / 2;
        (x, y)
    });

    // 别让桌宠跑出屏幕右边缘
    let max_x = mp.x + sz.width as i32 - pet_sz.width as i32 - 8;
    if x > max_x {
        x = max_x;
    }
    let _ = pet.set_position(PhysicalPosition::new(x, y));
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            persisted: Mutex::new(Persisted::default()),
            last_notify: Mutex::new(0),
            data_path: Mutex::new(std::path::PathBuf::new()),
            pet_saved_pos: Mutex::new(None),
            last_drag_epoch: Mutex::new(0),
            snooze_until: Mutex::new(0),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_stats,
            drink,
            undo_drink,
            snooze,
            save_config,
            toggle_pause,
            show_window,
            hide_window,
            switch_window,
            toggle_pet_visible,
            pet_retract,
            pet_expand,
            pet_go_home,
            pet_touched,
            reset_today,
            mark_onboarding,
            quit_app
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Regular 模式：Dock 显示图标，通知点击才能激活 App 触发 Reopen（进而弹主面板）。
            // 曾用 Accessory 隐藏 Dock 图标，但那样通知点了没反应；权衡后选可用性>无 Dock。
            #[cfg(target_os = "macos")]
            let _ = app.handle().set_activation_policy(tauri::ActivationPolicy::Regular);

            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let path = data_dir.join("state.json");
            let loaded = Persisted::load(&path);
            {
                let state = app.state::<AppState>();
                *state.data_path.lock().unwrap() = path.clone();
                let mut p = state.persisted.lock().unwrap();
                *p = loaded;
                clamp_config(&mut p.config);
                ensure_today(&mut p);
                migrate_legacy(&mut p);
                p.save(&path);
            }

            allow_pet_over_fullscreen(&handle);
            position_pet_beside_panel(&handle);

            // 托盘菜单
            let open_i = MenuItemBuilder::with_id("open", "打开面板").build(app)?;
            let toggle_pet_i =
                MenuItemBuilder::with_id("toggle_pet", "显示 / 隐藏柚柚").build(app)?;
            let home_i = MenuItemBuilder::with_id("home", "柚柚回家（找不到时点这个）").build(app)?;
            let pause_i = MenuItemBuilder::with_id("pause", "暂停 / 继续提醒").build(app)?;
            let settings_i = MenuItemBuilder::with_id("settings", "设置…").build(app)?;
            let quit_i = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[
                    &open_i,
                    &toggle_pet_i,
                    &home_i,
                    &pause_i,
                    &settings_i,
                    &quit_i,
                ])
                .build()?;

            let mut tray_builder = TrayIconBuilder::new().menu(&menu).on_menu_event(
                move |app, event| match event.id().as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "toggle_pet" => {
                        if let Some(w) = app.get_webview_window("pet") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                            }
                        }
                    }
                    "home" => {
                        let state = app.state::<AppState>();
                        *state.pet_saved_pos.lock().unwrap() = None;
                        if let Some(w) = app.get_webview_window("pet") {
                            let _ = w.show();
                            reset_pet_position_impl(&w);
                        }
                    }
                    "pause" => {
                        {
                            let state = app.state::<AppState>();
                            let mut p = state.persisted.lock().unwrap();
                            p.config.paused = !p.config.paused;
                            ensure_today(&mut p);
                            let path = state.data_path.lock().unwrap().clone();
                            p.save(&path);
                        }
                        emit_update(app);
                    }
                    "settings" => {
                        if let Some(w) = app.get_webview_window("settings") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                },
            );
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let _tray = tray_builder.build(app)?;

            let h_watch = app.handle().clone();
            std::thread::spawn(move || {
                let mut was_fullscreen = false;
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let pet_win = match h_watch.get_webview_window("pet") {
                        Some(w) => w,
                        None => continue,
                    };
                    if !pet_win.is_visible().unwrap_or(false) {
                        continue;
                    }
                    let mon = match pet_win.primary_monitor() {
                        Ok(Some(m)) => m,
                        _ => continue,
                    };
                    let sz = mon.size();
                    let mp = mon.position();
                    let scale = pet_win.scale_factor().unwrap_or(1.0);
                    let sw = sz.width as f64 / scale;
                    let sh = sz.height as f64 / scale;
                    let sx = mp.x as f64 / scale;
                    let sy = mp.y as f64 / scale;
                    let obs = screenscan::other_app_window_rects(std::process::id() as i64);
                    // 真全屏的窗口连菜单栏那条也盖住，尺寸和屏幕基本相等。
                    // 不能只看面积占比：最大化的普通窗口面积也很大，但它在菜单栏下面，
                    // 那种情况桌宠明明看得见，不该缩到边上去。
                    let has_fs = obs.iter().any(|&(x, y, w, h)| {
                        w >= sw - 2.0 && h >= sh - 2.0 && y <= sy + 1.0 && x <= sx + 1.0
                    });

                    if has_fs && !was_fullscreen {
                        was_fullscreen = true;
                        let win_sz = pet_win.outer_size().unwrap_or_default();
                        let cur = pet_win.outer_position().ok();
                        let state = h_watch.state::<AppState>();
                        {
                            let mut saved = state.pet_saved_pos.lock().unwrap();
                            if saved.is_none() {
                                if let Some(p) = cur {
                                    *saved = Some((p.x, p.y));
                                }
                            }
                        }
                        let retract_x = mp.x + sz.width as i32 - 80;
                        let y = mp.y + sz.height as i32 - win_sz.height as i32 - 120;
                        let _ = pet_win.set_position(PhysicalPosition::new(retract_x, y));
                        let _ = h_watch.emit("pet-retract", ());
                    } else if !has_fs && was_fullscreen {
                        was_fullscreen = false;
                        let state = h_watch.state::<AppState>();
                        let saved = state.pet_saved_pos.lock().unwrap().take();
                        if let Some((x, y)) = saved {
                            let _ = pet_win.set_position(PhysicalPosition::new(x, y));
                        } else {
                            reset_pet_position_impl(&pet_win);
                        }
                        let _ = h_watch.emit("pet-expand", ());
                    }
                }
            });

            // 提醒引擎
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                let state = handle.state::<AppState>();
                let (remind, snap) = {
                    let mut p = state.persisted.lock().unwrap();
                    let reset = ensure_today(&mut p);
                    let last_notify = *state.last_notify.lock().unwrap();
                    let snooze = *state.snooze_until.lock().unwrap();
                    let interval = (p.config.interval_minutes as i64) * 60;
                    let remind =
                        remind_due(should_remind(&p), now_epoch(), last_notify, snooze, interval);
                    (remind, build_snapshot(&p, reset))
                };
                if remind {
                    *state.last_notify.lock().unwrap() = now_epoch();
                    *state.snooze_until.lock().unwrap() = 0; // 稍后已兑现，清掉
                    let sound_on = {
                        let p = state.persisted.lock().unwrap();
                        p.config.sound_enabled
                    };
                    let mut b = handle
                        .notification()
                        .builder()
                        .title("该喝水啦")
                        .body("起身接杯水，陪柚柚喝一口。");
                    if sound_on {
                        b = b.sound("default");
                    }
                    let _ = b.show();
                    let _ = handle.emit("pet-alert", ());
                }
                let _ = handle.emit("updated", snap);
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            // 用户点系统通知 / 菜单栏图标激活 App 时，把主面板浮到前面。
            // Accessory 模式下没有 Dock 图标，Reopen 是 App 被从系统层激活的通用信号。
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(w) = app_handle.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Persisted {
        let mut p = Persisted::default();
        p.day.date = today_str();
        p.config.cup_ml = 300;
        p.config.daily_goal_ml = 2000;
        p
    }

    // 四个字段（count / total_ml / last_drink_epoch / drink_log）必须始终自洽
    fn assert_coherent(p: &Persisted) {
        assert_eq!(
            p.day.count as usize,
            p.day.drink_log.len(),
            "count 必须等于 drink_log 长度"
        );
        let sum: u32 = p.day.drink_log.iter().map(|e| e.ml).sum();
        assert_eq!(p.day.total_ml, sum, "total_ml 必须等于 drink_log 各条之和");
        let expect_epoch = p.day.drink_log.last().map(|e| e.epoch).unwrap_or(0);
        assert_eq!(
            p.day.last_drink_epoch, expect_epoch,
            "last_drink_epoch 必须等于最后一条的时刻（空则为0）"
        );
    }

    // 守护「下次喝水」计时：喝水后基准往后推、撤销后退回。活跃时段设全天避免被非活跃态拦截。
    #[test]
    fn next_label_喝水推后撤销退回() {
        let mut p = fresh();
        p.config.active_start = "00:00".into();
        p.config.active_end = "23:59".into();
        p.config.interval_minutes = 60;
        let t = now_epoch();
        apply_drink(&mut p, Some(200), t);
        let l1 = next_label(&p);
        apply_drink(&mut p, Some(200), t + 1800); // 30 分钟后第二杯
        let l2 = next_label(&p);
        apply_undo(&mut p); // 撤销第二杯
        let l3 = next_label(&p);
        println!("第一杯基准={l1}  第二杯基准={l2}  撤销后={l3}");
        assert_ne!(l1, l2, "第二杯喝水后下次时间应往后推 30 分钟");
        assert_eq!(l1, l3, "撤销后下次时间应退回上一杯的基准");
    }

    #[test]
    fn drink_默认用杯子容量() {
        let mut p = fresh();
        apply_drink(&mut p, None, 1000);
        assert_eq!(p.day.total_ml, 300);
        assert_eq!(p.day.count, 1);
        assert_coherent(&p);
    }

    #[test]
    fn drink_自定义量() {
        let mut p = fresh();
        apply_drink(&mut p, Some(50), 1000);
        apply_drink(&mut p, Some(777), 2000);
        assert_eq!(p.day.total_ml, 827);
        assert_eq!(p.day.count, 2);
        assert_eq!(p.day.last_drink_epoch, 2000);
        assert_coherent(&p);
    }

    #[test]
    fn undo_逐条回退且状态自洽() {
        let mut p = fresh();
        apply_drink(&mut p, Some(100), 1000);
        apply_drink(&mut p, Some(250), 2000);
        apply_drink(&mut p, Some(50), 3000);
        assert_coherent(&p);

        assert!(apply_undo(&mut p));
        assert_eq!(p.day.total_ml, 350);
        assert_eq!(p.day.last_drink_epoch, 2000, "撤销后计时基准回退到前一杯");
        assert_coherent(&p);

        assert!(apply_undo(&mut p));
        assert_eq!(p.day.total_ml, 100);
        assert_eq!(p.day.last_drink_epoch, 1000);
        assert_coherent(&p);
    }

    #[test]
    fn undo_可以无限撤到空() {
        let mut p = fresh();
        for i in 0..5 {
            apply_drink(&mut p, Some(100), 1000 + i);
        }
        for _ in 0..5 {
            assert!(apply_undo(&mut p));
        }
        assert_eq!(p.day.count, 0);
        assert_eq!(p.day.total_ml, 0);
        assert_eq!(p.day.last_drink_epoch, 0);
        assert_coherent(&p);
    }

    #[test]
    fn undo_空记录时不动任何字段() {
        let mut p = fresh();
        assert!(!apply_undo(&mut p), "空 log 撤销应返回 false");
        assert_eq!(p.day.count, 0);
        assert_eq!(p.day.total_ml, 0);
        assert_coherent(&p);
    }

    #[test]
    fn drink_undo_交错() {
        let mut p = fresh();
        apply_drink(&mut p, Some(300), 1000);
        apply_undo(&mut p);
        apply_drink(&mut p, Some(500), 2000);
        apply_drink(&mut p, Some(200), 3000);
        apply_undo(&mut p);
        apply_drink(&mut p, Some(50), 4000);
        assert_eq!(p.day.total_ml, 550);
        assert_eq!(p.day.count, 2);
        assert_eq!(p.day.last_drink_epoch, 4000);
        assert_coherent(&p);
    }

    #[test]
    fn 里程碑_过半只报一次() {
        let mut p = fresh(); // goal 2000, half 1000
        assert_eq!(apply_drink(&mut p, Some(900), 1000), None);
        assert_eq!(apply_drink(&mut p, Some(200), 2000), Some("half"), "1100 跨过半程");
        assert_eq!(apply_drink(&mut p, Some(100), 3000), None, "已过半不再报");
    }

    #[test]
    fn 里程碑_达标() {
        let mut p = fresh();
        apply_drink(&mut p, Some(1900), 1000);
        assert_eq!(apply_drink(&mut p, Some(200), 2000), Some("goal"));
        assert_eq!(apply_drink(&mut p, Some(300), 3000), None, "达标后不再报");
    }

    #[test]
    fn 里程碑_一杯同时跨过半和达标时报达标() {
        let mut p = fresh();
        assert_eq!(apply_drink(&mut p, Some(2000), 1000), Some("goal"));
    }

    #[test]
    fn 里程碑_撤销后可以重新报() {
        let mut p = fresh();
        apply_drink(&mut p, Some(1900), 1000);
        assert_eq!(apply_drink(&mut p, Some(200), 2000), Some("goal"));
        apply_undo(&mut p);
        assert_eq!(apply_drink(&mut p, Some(200), 3000), Some("goal"), "撤销达标那杯再喝应重新报达标");
    }

    #[test]
    fn 跨天清空所有字段() {
        let mut p = fresh();
        apply_drink(&mut p, Some(500), 1000);
        p.day.date = "1999-01-01".into();
        assert!(ensure_today(&mut p), "日期不同应返回 true");
        assert_eq!(p.day.count, 0);
        assert_eq!(p.day.total_ml, 0);
        assert_eq!(p.day.last_drink_epoch, 0);
        assert!(p.day.drink_log.is_empty());
        assert_coherent(&p);
    }

    #[test]
    fn 迁移_老数据只有count时补出total和log() {
        let mut p = fresh();
        p.day.count = 3;
        p.day.total_ml = 0;
        p.day.last_drink_epoch = 5000;
        migrate_legacy(&mut p);
        assert_eq!(p.day.total_ml, 900, "3 杯 x 300ml");
        assert_eq!(p.day.drink_log.len(), 3);
        assert_coherent(&p);
    }

    #[test]
    fn 迁移_有total但无log的老数据() {
        let mut p = fresh();
        p.day.count = 2;
        p.day.total_ml = 350; // 用户当时记的是自定义量，不是 2x300
        p.day.last_drink_epoch = 5000;
        migrate_legacy(&mut p);
        assert_coherent(&p);
    }

    #[test]
    fn 迁移_新数据不受影响() {
        let mut p = fresh();
        apply_drink(&mut p, Some(123), 1000);
        let before = p.day.total_ml;
        migrate_legacy(&mut p);
        assert_eq!(p.day.total_ml, before, "已有 log 的数据不该被迁移改动");
        assert_eq!(p.day.drink_log.len(), 1);
        assert_coherent(&p);
    }

    #[test]
    fn snooze_到点前不提醒到点后提醒() {
        let now = 100_000;
        let interval = 3600;
        // 点了待会儿 → snooze = now + 900（15 分钟）
        let snooze = now + 900;
        // 稍后期间（还没到 15 分钟）：不提醒，哪怕常规间隔早过了
        assert!(!remind_due(true, now + 300, 0, snooze, interval));
        assert!(!remind_due(true, now + 899, 0, snooze, interval));
        // 到点（满 15 分钟）：提醒
        assert!(remind_due(true, now + 900, 0, snooze, interval));
        assert!(remind_due(true, now + 1000, 0, snooze, interval));
    }

    #[test]
    fn snooze_基本条件不满足时仍不提醒() {
        let now = 100_000;
        // 就算稍后到点了，base_ok=false（比如已达标/暂停/不在活跃时段）也不提醒
        assert!(!remind_due(false, now + 1000, 0, now + 900, 3600));
    }

    #[test]
    fn 无稍后时按常规间隔提醒() {
        let interval = 3600;
        let last_notify = 100_000;
        // 没到间隔：不提醒
        assert!(!remind_due(true, last_notify + 3599, last_notify, 0, interval));
        // 到间隔：提醒
        assert!(remind_due(true, last_notify + 3600, last_notify, 0, interval));
    }

    #[test]
    fn 撤到空后提醒引擎不会立刻炸() {
        let mut p = fresh();
        p.config.active_start = "00:00".into();
        p.config.active_end = "23:59".into();
        apply_drink(&mut p, Some(100), now_epoch());
        assert!(!should_remind(&p), "刚喝完不该提醒");
        apply_undo(&mut p);
        // 撤到空 last_drink_epoch=0，此时 should_remind 用 now-interval 兜底
        let _ = should_remind(&p);
    }

    #[test]
    fn 归档_跨天把喝过的那天存进history() {
        let mut p = fresh();
        apply_drink(&mut p, Some(1800), 1000);
        p.day.date = "2026-07-19".into();
        p.config.daily_goal_ml = 2000;
        ensure_today(&mut p);
        assert_eq!(p.history.len(), 1);
        assert_eq!(p.history[0].date, "2026-07-19");
        assert_eq!(p.history[0].total_ml, 1800);
        assert_eq!(p.history[0].goal_ml, 2000, "归档存的是当天目标");
        // 归档后当天字段照旧清空
        assert_eq!(p.day.total_ml, 0);
        assert_coherent(&p);
    }

    #[test]
    fn 归档_没喝过的天不归档() {
        let mut p = fresh();
        p.day.date = "2026-07-19".into(); // total_ml 仍是 0
        ensure_today(&mut p);
        assert!(p.history.is_empty(), "没喝过的天不该进 history");
    }

    #[test]
    fn 归档_同一天幂等不重复push() {
        let mut p = fresh();
        p.history.push(DayStat {
            date: "2026-07-19".into(),
            total_ml: 500,
            goal_ml: 2000,
        });
        // 手工造一个 day 停在已归档的那天
        p.day.date = "2026-07-19".into();
        p.day.total_ml = 500;
        archive_day(&mut p);
        assert_eq!(p.history.len(), 1, "同一天不重复归档");
    }

    #[test]
    fn 归档_超过30天裁掉最老的() {
        let mut p = fresh();
        for i in 0..30 {
            p.history.push(DayStat {
                date: format!("2026-06-{:02}", i + 1),
                total_ml: 1000,
                goal_ml: 2000,
            });
        }
        // 归档第 31 天
        p.day.date = "2026-07-19".into();
        p.day.total_ml = 1800;
        archive_day(&mut p);
        assert_eq!(p.history.len(), 30, "裁剪到 30 条");
        assert_eq!(p.history[0].date, "2026-06-02", "最老的一条被丢掉");
        assert_eq!(p.history.last().unwrap().date, "2026-07-19");
    }

    #[test]
    fn 统计_按日期轴补齐缺口天为0() {
        let mut p = fresh();
        p.day.date = "2026-07-19".into();
        p.day.total_ml = 800;
        p.config.daily_goal_ml = 2000;
        p.history.push(DayStat {
            date: "2026-07-17".into(),
            total_ml: 2100,
            goal_ml: 2000,
        });
        // 缺 07-18
        let dates = vec![
            "2026-07-17".to_string(),
            "2026-07-18".to_string(),
            "2026-07-19".to_string(),
        ];
        let pts = assemble_stats(&p, &dates);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].total_ml, 2100);
        assert_eq!(pts[0].goal_ml, 2000);
        assert!(!pts[0].is_today);
        assert_eq!(pts[1].total_ml, 0, "缺口天补 0");
        assert_eq!(pts[1].goal_ml, 2000, "缺口天目标用当前配置");
        assert_eq!(pts[2].total_ml, 800);
        assert!(pts[2].is_today, "最后一天是今天");
    }

    #[test]
    fn 统计_今天优先取实时数据不看history() {
        let mut p = fresh();
        p.day.date = "2026-07-19".into();
        p.day.total_ml = 1200;
        // 就算 history 里意外也有今天，也以 day 为准
        p.history.push(DayStat {
            date: "2026-07-19".into(),
            total_ml: 999,
            goal_ml: 2000,
        });
        let dates = vec!["2026-07-19".to_string()];
        let pts = assemble_stats(&p, &dates);
        assert_eq!(pts[0].total_ml, 1200, "今天取 day.total_ml 而非 history");
        assert!(pts[0].is_today);
    }
}
