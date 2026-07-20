# 项目：喝水提醒桌宠（Mac 桌面程序）

## 这个项目现在是什么

面向久坐不喝水的上班族的 Mac 桌面程序。桌面上常驻一个桌宠「柚柚」（头顶柚子的水豚），到点提醒喝水，点一下记一杯。

**当前在做的是 v2（Tauri 版），代码在 `app/`。改代码默认就是改这里。**

用户是零编程基础的小白。跟他沟通说人话，交付物要能被非技术用户直接用。

## 历史：曾有个 v1（Python 版），已删除

早期有个用 Python + rumps 写的菜单栏版本（`main.py`、`启动提醒.command`），2026-07-19 定稿 v2 后已删除，需要时可从 git 历史 v1.0 标签之前找回。现在仓库里只有 v2，不要再参考 rumps 那套实现。

## v2 技术栈与约束

- **框架**：Tauri v2。Rust 后端 + 静态 HTML/CSS/JS 前端。
- **没有 Node.js、没有构建步骤、没有前端框架**。`frontendDist` 直接指向 `../src`，HTML 单文件内联 `<style>` 和 `<script>`，改完刷新即可。别引入 npm/打包器。
- **没有 CSS 变量**（`:root` 那套）。所有颜色/间距硬编码在各 HTML 文件里。`docs/ui-20260712-water-reminder-v2.md` 定义了 token 名但代码没实现，照现状用硬编码值。
- **通知**：`tauri-plugin-notification`（不是 v1 的 osascript）。
- **开机自启**：`tauri-plugin-autostart`，macOS 走 LaunchAgent。
- **窗口位置探测**：`core-graphics` 的 `CGWindowListCopyWindowInfo`，只读窗口边框矩形，**不需要辅助功能/屏幕录制授权**。
- 运行环境：macOS，Apple Silicon。

## v2 文件结构

```
app/
  src/                      前端（frontendDist 指向这里）
    main/index.html         主面板 340x620
    pet/index.html          桌宠 240x260
    settings/index.html     设置页 340x620
  src-tauri/
    src/main.rs             命令、托盘、提醒引擎、后台线程
    src/model.rs            Config / DayState / DrinkEntry / AppState
    src/screenscan.rs       读其他 app 窗口位置
    src/timeutil.rs         时间工具
    tauri.conf.json         三个窗口的配置
    capabilities/default.json  权限声明（加插件要在这里加权限）
docs/                       设计方案与 spec
```

三个窗口都是 transparent + 无边框。pet 窗口 alwaysOnTop + visibleOnAllWorkspaces + skipTaskbar。settings 默认 visible=false。

**数据落盘**：`~/Library/Application Support/com.waterreminder.pet/state.json`（`app_data_dir()/state.json`），不在项目目录。

## 怎么跑 / 怎么测

```bash
cd app/src-tauri
cargo tauri dev        # 起 app（原生窗口，不是网页）
cargo test             # 跑 Rust 单测
```

**前端没法直接在浏览器里开**：`index.html` 第一行就是 `window.__TAURI__.core`，没有 Tauri 桥会直接抛错，整个脚本不执行，你会看到一个死页面（滚轮空、按钮无反应）。要在浏览器里测前端，必须先注入 `window.__TAURI__` 的桩再加载页面。

**测布局别靠 CSS 算术**：起个 `python3 -m http.server` 服务，用 `getBoundingClientRect()` 量真实位置。照着 margin 值加减算出来的数跟实际对不上，这个坑踩过两次。

**无头/后台浏览器的两个陷阱**（都误导过判断）：
- `focus()` 不触发 focus 事件（`document.hasFocus()` 为 false），依赖 focus/blur 的逻辑测不出来
- `visibilityState: hidden` 时 `requestAnimationFrame` 不执行，rAF 里的逻辑整段跑不到

## 关键实现要点（改代码前必看）

- **`drink_log` 是唯一真相**。`count` 由 `drink_log.len()` 派生，`total_ml` 是各条之和，`last_drink_epoch` 是最后一条的时刻。这四个字段必须始终自洽，别让它们各自维护。改动线记录逻辑后跑 `cargo test`，测试里有 `assert_coherent` 专门守这条不变式。
- **纯状态变更抽在 `apply_drink` / `apply_undo` / `migrate_legacy` 里**，不依赖 `AppHandle`，所以能单测。`#[tauri::command]` 的 `drink` / `undo_drink` 只是薄壳。新增状态逻辑照这个分法写，别塞回命令里，否则又没法测。
- **老数据迁移要保证 `sum(drink_log) == total_ml`**。曾经按 `count * cup_ml` 凭空造记录，跟真实 `total_ml` 对不上，撤销会扣出错误数字。现在是把 `total_ml` 按次数摊回去、余数给最后一条。
- **任意入口打卡都要重置提醒计时**。计时记的是「距上次喝水多久」，不是「距上次提醒多久」。三个入口（点桌宠、通知、面板按钮）任一次 +1 都要清零倒计时。
- **加 Tauri 插件必须同时改两处**：`Cargo.toml` 加依赖 + `capabilities/default.json` 加权限，只加一处会在运行时静默失败。
- **前端 `invoke` 失败是静默的**。前端调后端命令出错不会弹任何东西，写的时候要自己 catch 并给用户反馈。
- **自定义饮水量面板的取值规则是「最后动的那个赢」**：碰滚轮 → 滚轮回写输入框；点进输入框 → 滚轮停止回写。用 `mlWheelDriving` 一个标志控制。这里改动过三轮，别退回「用 focus/blur 判断谁说了算」的写法，会打架。
- 通知文案里不要出现英文双引号。

## 产品决策（已定，别自己改）

**形象**：头顶柚子的佛系水豚，叫「柚柚」。勋章形状 = 柚子。四状态姿势见 `docs/ui-20260712-water-reminder-v2.md` 第 2.4 节。

**三个打卡入口，记同一份今日计数**：
1. 提醒态：到点桌宠蹦出来，气泡点「喝了」= +1；「待会儿」不加、稍后再提醒
2. 主动打卡：点待机桌宠 → 弹气泡确认（不做「点一下直接 +1」，防手滑）
3. 面板的「喝了一杯」按钮

**聪明提醒**：只在活跃时段（默认 9:00-18:00）提醒；距上次喝水 ≥ 设定间隔（默认 60 分钟）且今天未达标才提醒；喝了立即重置计时。达标 / 下班 / 暂停 / 免打扰都不提醒。

**桌宠四状态**：待机（要活泼耐看，别呆滞）、提醒、开心（记一杯后庆祝一两秒）、睡觉（暂停/下班/免打扰）。**达标不算睡觉**，桌宠醒着仍可点记超额。**睡觉发灰只用于暂停/免打扰；下班（offhours）保持原色不发灰**（用户要求下班和上班颜色一致），Zzz 标记和眯眼仍保留。逻辑在 pet/index.html 的 `face()`：`dulled = sleeping && reason !== 'offhours'`。

**记录量**：以 ml 为准，不是杯数。可记整杯 / 半杯 / 一小口 50ml / 自定义（滚动选择器 + 手输，10-2000ml）。撤销走 `drink_log` 可无限撤，按钮常驻、没记录时置灰。

**达标处收住**：超过目标照记数，不再发新勋章，文案转平和「今天喝得很足」，主按钮退成低调灰底「再来一杯」。理由：水喝够就行，不奖励越喝越多。

**桌宠避让（已收敛为方案 A）**：只保留「其他 app 全屏时桌宠缩到屏幕边缘露个头」。**打开面板时不再自动挪桌宠**（改过几轮，效果都不好，用户觉得乱跑不受控）。挡住了让用户手动拖，或用菜单栏「柚柚回家」。

因此 `pet_avoid_windows` → `do_avoid` → `screenscan::find_free_spot` 整条链现在没有调用方，是死代码。用户要求先留着不删（以后进 git 再规范处理）。它注册在 `generate_handler!` 里所以编译器不报 dead_code 警告，别以为它还活着。

## 状态

**已做**：桌宠常驻 + 四状态、三入口打卡、聪明提醒、设置页（间隔/活跃时段/杯容量/每日目标/免打扰/提示音）、ml 记录 + 自定义量、无限撤销、全屏藏边、开机自启动、喝水统计（单独 stats 窗口 + 主面板柱状图入口，留 30 天历史，日均/连续达标/达标率三卡 + 近7天柱状图/近30天平滑曲线）。

**喝水统计实现要点**：`drink_log` 只管当天；跨天时 `ensure_today` 调 `archive_day` 把结束的那天摊成一条 `DayStat{date,total_ml,goal_ml}` 存进 `Persisted.history`（只归档喝过的天、按 date 幂等去重、裁到 30 条）。`goal_ml` 存当天目标，判历史达标用它不用今天的目标。`get_stats` 命令返回近 30 天连续日期轴（缺口补 0，今天取实时 day 数据），日均/连续/达标率全在前端 `stats/index.html` 算。加窗口记得三处：tauri.conf.json + capabilities/default.json 的 windows + generate_handler 注册命令。

**没验证过**：**通知从没在打包成 .app 之后验证过**。`cargo tauri dev` 下的通知行为跟正式 .app 可能不同（签名、通知权限）。这是当前最大的未知，优先做。

**暂缓**：站立提醒（暂不做）。

**代码管理**：已进 git + GitHub（`git@github.com:J-kea-ton/water-reminder.git`），走 SSH。改完让本体跑 add/commit/push。

**版本管理**：`tauri.conf.json` 的 `version` 是唯一版本号来源，改功能要同步升它（语义化版本：加功能升次版本号，修 bug 升修订号），并打对应 git tag（`vX.Y.Z`）推到云端。历史标签：v1.0 = 2026-07-19 定稿版（当时 version 仍是 0.1.0，标签号和 version 不一致，属历史遗留）；**当前 v0.2.0**（2026-07-21，含喝水统计 + 桌宠下班保持原色 + 窗口切换保持原位，version 与 tag 已对齐，往后都保持一致）。

## 写作与沟通约定

- 默认中文、结论先行、少术语；绕不开术语时顺带一句人话解释。
- 遵守全局 `~/.claude/rules/no_ai_style.md`（禁 AI 腔）。
- 中文与数字/英文之间不加空格。
