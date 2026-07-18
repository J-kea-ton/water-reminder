use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

// 持久化配置（对应设置页各项）。字段用 camelCase 与前端 JS 对齐。
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub interval_minutes: u32,
    pub active_start: String,
    pub active_end: String,
    pub cup_ml: u32,
    pub daily_goal_ml: u32,
    pub paused: bool,
    pub dnd_enabled: bool,
    pub dnd_start: String,
    pub dnd_end: String,
    pub sound_enabled: bool,
    pub onboarding_shown: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_minutes: 60,
            active_start: "09:00".into(),
            active_end: "18:00".into(),
            cup_ml: 300,
            daily_goal_ml: 2000,
            paused: false,
            dnd_enabled: false,
            dnd_start: "12:00".into(),
            dnd_end: "14:00".into(),
            sound_enabled: true,
            onboarding_shown: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrinkEntry {
    pub ml: u32,
    pub epoch: i64,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DayState {
    pub date: String,
    pub count: u32,
    pub total_ml: u32,
    pub last_drink_epoch: i64,
    pub drink_log: Vec<DrinkEntry>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Persisted {
    pub config: Config,
    pub day: DayState,
}

impl Persisted {
    pub fn load(path: &PathBuf) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, s);
        }
    }
}

// 运行时全局状态。last_notify 只存进程内（重启后允许恢复提醒）。
pub struct AppState {
    pub persisted: Mutex<Persisted>,
    pub last_notify: Mutex<i64>,
    pub data_path: Mutex<PathBuf>,
    // 桌宠缩到边前的原始位置，展开时用来还原
    pub pet_saved_pos: Mutex<Option<(i32, i32)>>,
    // 用户最后一次拖动桌宠的时刻，刚拖完的几秒内不自动挪，免得跟手打架
    pub last_drag_epoch: Mutex<i64>,
    // 点了「待会儿」后的稍后提醒时刻；0 表示没在稍后中。到点且仍该提醒时再弹一次。
    pub snooze_until: Mutex<i64>,
}
