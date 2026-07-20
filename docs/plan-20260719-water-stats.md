# 喝水统计功能 - 实现方案

- 日期：20260719
- 状态：原型已定稿（用户确认），待实现。动代码前本文件是唯一权威。
- 原型长相：单独一页「喝水统计」，见下方"页面规格"。

## 用户已确认的决策

1. **留 30 天历史**（滚动，超过 30 天丢最老的）
2. **只记每天总量**（不留每一杯的时间点，历史里一天一个数）
3. **单独开一页**，叫「喝水统计」，不塞进主面板
4. **入口**：主面板顶部栏，日期旁边加一个柱状图小图标，点它进统计页
5. **三张数字卡**：日均 / 连续达标 / 达标率（各说一件事，不重复）
6. **近 7 天柱状图**（绿=达标/蓝=没达标/浅灰=没喝），**近 30 天平滑曲线**
7. **切换栏**：下划线式标签（不要色块轨道），蓝线滑动
8. **鼓励横幅**：图表下方居中一条软橙药丸（🔥 已经连续 N 天喝够啦）

## 数据存储改动

现状：`Persisted { config, day }`，`day` 只存当天，跨天被 `ensure_today` 清空，不留历史。

**改法**（model.rs）：

```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStat {
    pub date: String,      // "2026-07-19"
    pub total_ml: u32,     // 当天总量
    pub goal_ml: u32,      // 当天的目标（目标可能被改过，存当时的）
}

// Persisted 加一个字段
pub struct Persisted {
    pub config: Config,
    pub day: DayState,
    pub history: Vec<DayStat>,   // 已完成的每日总量，最多 30 条，旧的在前
}
```

- `#[serde(default)]` 已在 Persisted 上，老 state.json 没有 history 字段会自动填空 Vec，不会崩。**迁移零成本**。

## 后端改动（main.rs）

### 1. 跨天时归档（关键）

`ensure_today` 现在跨天直接清空 `day`。改成：**清空前，把要结束的那天存进 history**。

```
fn ensure_today(p) {
    if p.day.date != today {
        // 归档：只归档真喝过水的天（total_ml>0 或 date 非空且有记录）
        if !p.day.date.is_empty() && p.day.total_ml > 0 {
            p.history.push(DayStat {
                date: p.day.date, total_ml: p.day.total_ml,
                goal_ml: p.config.daily_goal_ml,  // 用当天的目标
            });
            // 去重（同一天只留一条，防重复归档）
            // 裁剪到最近 30 条
            if p.history.len() > 30 { p.history.drain(0..len-30); }
        }
        // ...原来的清空逻辑照旧
    }
}
```

注意：`ensure_today` 在多处被调用（drink/undo/get_snapshot/提醒线程），归档逻辑要幂等——同一天不重复 push。用 date 判重。

### 2. 新命令 get_stats

给统计页用。返回最近 30 天的连续数组（缺的天补 0）+ 今天的实时总量 + 目标。

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsPoint { date: String, total_ml: u32, goal_ml: u32, is_today: bool }

#[tauri::command]
fn get_stats(state) -> Vec<StatsPoint> {
    // 1. ensure_today（触发可能的归档）
    // 2. 取 history（已完成的天）+ 追加今天（day.total_ml, config.daily_goal_ml, is_today=true）
    // 3. 按日期补齐：从 29 天前到今天，逐天查，没有记录的补 total_ml=0
    //    （用 chrono 按日期减，timeutil 里已有时间工具，可能要加"日期往前推 N 天"的辅助）
    // 4. 返回 30 个点（或不足 30 天就返回实际天数）
}
```

前端拿到这 30 个点后，自己切 7/30、算日均/连续达标/达标率。**统计逻辑放前端算**（跟原型一致，省得后端前端两头维护）。

- 连续达标：从今天往前数，连着 total_ml>=goal_ml 的天数，断在第一个没达标的天。
- 日均：只算 total_ml>0 的天（没喝的天不拉低）。
- 达标率：达标天数 / 视图天数（7 或 30）。

### 3. 注册命令

`get_stats` 加进 `generate_handler!`。

## 新窗口（tauri.conf.json）

加第四个窗口 `stats`，跟 settings 一样：340x620、transparent、无边框、visible=false、默认隐藏。
`capabilities/default.json` 的 windows 列表加 "stats"。

## 新前端页 app/src/stats/index.html

- 拿原型（docs 里 water_stats_page_prototype_v4 的样式和图表逻辑）改成真页面。
- 开头 `invoke('get_stats')` 取数据，`listen('updated')` 时刷新。
- 返回按钮：`hide_window('stats')` + `show_window('main')`（照 settings 页的返回逻辑）。
- 图表用 SVG（原型已有 bars() 柱状图 + line() 平滑曲线，直接搬）。
- 页面第一行有 `window.__TAURI__.core`，浏览器直接开会崩，测试要注入桩（见 CLAUDE.md）。

## 主面板改动 app/src/main/index.html

顶部栏日期旁加统计图标（柱状图 icon），点击：
```
await invoke('hide_window',{label:'main'});
await invoke('show_window',{label:'stats'});
```
（跟齿轮进设置一个套路）

## 测试

- 后端：归档逻辑抽成纯函数测（跨天归档、去重幂等、裁剪到30、只归档喝过的天）。补齐日期数组的逻辑也纯函数测（有缺口补0）。放 main.rs #[cfg(test)]。
- 前端：注入 get_stats 桩，喂各种历史数据，验证 7/30 切换、日均/连续/达标率算对、曲线不崩。
- 跑 `cargo test` + `cargo tauri build` 出包。

## 潜在坑

- **归档幂等**：ensure_today 一天内被调多次，别重复 push 同一天。用 date 判重（push 前查 history 最后一条 date 是不是要归档的 date）。
- **目标改过**：达标看的是"当天的目标"，所以 DayStat 要存 goal_ml，不能用今天的目标去判历史。
- **日期推算**：补齐 30 天连续数组需要"某日期往前推 N 天"，timeutil 可能要加个辅助函数。
- **时区**：都用本地日期（today_str() 已经是本地），保持一致。
