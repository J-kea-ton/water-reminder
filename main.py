#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
喝水 & 站立 提醒 —— Mac 菜单栏小程序

启动后屏幕右上角会出现一个 💧 图标，到点用系统通知提醒你喝水、站起来。
点开图标可以：修改提醒间隔、暂停/继续、立即测试、退出。
"""

import json
import os
import subprocess

import rumps


# ---------- 配置读写 ----------
CONFIG_DIR = os.path.expanduser("~/Library/Application Support/WaterStandReminder")
CONFIG_PATH = os.path.join(CONFIG_DIR, "config.json")

DEFAULT_CONFIG = {
    "water_minutes": 30,   # 喝水提醒间隔（分钟）
    "stand_minutes": 60,   # 站立提醒间隔（分钟）
}


def load_config():
    """读配置；文件不存在或损坏就用默认值。"""
    try:
        with open(CONFIG_PATH, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return dict(DEFAULT_CONFIG)

    cfg = dict(DEFAULT_CONFIG)
    for key in DEFAULT_CONFIG:
        value = data.get(key)
        if isinstance(value, int) and value > 0:
            cfg[key] = value
    return cfg


def save_config(cfg):
    os.makedirs(CONFIG_DIR, exist_ok=True)
    with open(CONFIG_PATH, "w", encoding="utf-8") as f:
        json.dump(cfg, f, ensure_ascii=False, indent=2)


# ---------- 系统通知 ----------
def notify(title, message, sound="Glass"):
    """用 macOS 自带的方式弹一条带声音的系统通知。"""
    def esc(text):
        return text.replace("\\", "\\\\").replace('"', '\\"')

    script = (
        f'display notification "{esc(message)}" '
        f'with title "{esc(title)}" sound name "{sound}"'
    )
    subprocess.run(["osascript", "-e", script], check=False)


# ---------- 主程序 ----------
class ReminderApp(rumps.App):
    def __init__(self):
        super().__init__("喝水提醒", title="💧", quit_button=None)

        self.config = load_config()
        self.paused = False

        # 菜单项
        self.water_item = rumps.MenuItem("", callback=self.change_water)
        self.stand_item = rumps.MenuItem("", callback=self.change_stand)
        self.pause_item = rumps.MenuItem("⏸ 暂停提醒", callback=self.toggle_pause)
        self.test_item = rumps.MenuItem("🔔 立即测试一次", callback=self.test_now)

        self.menu = [
            self.water_item,
            self.stand_item,
            None,                                    # 分隔线
            self.pause_item,
            self.test_item,
            None,
            rumps.MenuItem("关于", callback=self.about),
            rumps.MenuItem("退出", callback=self.quit_app),
        ]

        # 两个独立计时器
        self.water_timer = rumps.Timer(self.on_water, self.config["water_minutes"] * 60)
        self.stand_timer = rumps.Timer(self.on_stand, self.config["stand_minutes"] * 60)

        self.refresh_menu_titles()
        self.water_timer.start()
        self.stand_timer.start()

    # ----- 刷新菜单文字 -----
    def refresh_menu_titles(self):
        self.water_item.title = f"💧 喝水提醒：每 {self.config['water_minutes']} 分钟（点击修改）"
        self.stand_item.title = f"🧍 站立提醒：每 {self.config['stand_minutes']} 分钟（点击修改）"

    # ----- 计时器回调 -----
    def on_water(self, _timer):
        notify("💧 该喝水啦", "起身接杯水，润润嗓子。", sound="Glass")

    def on_stand(self, _timer):
        notify("🧍 该站起来啦", "久坐伤身，起来动一动、伸个懒腰。", sound="Ping")

    # ----- 修改间隔 -----
    def _ask_minutes(self, what, current):
        """弹窗问用户新的分钟数；取消或非法输入返回 None。"""
        win = rumps.Window(
            message=f"每隔多少分钟提醒一次{what}？请输入一个正整数。",
            title="修改提醒间隔",
            default_text=str(current),
            ok="保存",
            cancel="取消",
            dimensions=(200, 24),
        )
        resp = win.run()
        if resp.clicked != 1:          # 1 = 保存；0 = 取消
            return None
        text = resp.text.strip()
        if not text.isdigit() or int(text) <= 0:
            rumps.alert("输入无效", "请输入一个大于 0 的整数（分钟）。")
            return None
        return int(text)

    def _restart_timer(self, timer, minutes):
        """安全地用新间隔重启计时器：先停、再改、再启。"""
        timer.stop()
        timer.interval = minutes * 60
        if not self.paused:
            timer.start()

    def change_water(self, _sender):
        minutes = self._ask_minutes("喝水", self.config["water_minutes"])
        if minutes is None:
            return
        self.config["water_minutes"] = minutes
        save_config(self.config)
        self.refresh_menu_titles()
        self._restart_timer(self.water_timer, minutes)

    def change_stand(self, _sender):
        minutes = self._ask_minutes("站立", self.config["stand_minutes"])
        if minutes is None:
            return
        self.config["stand_minutes"] = minutes
        save_config(self.config)
        self.refresh_menu_titles()
        self._restart_timer(self.stand_timer, minutes)

    # ----- 暂停 / 继续 -----
    def toggle_pause(self, _sender):
        self.paused = not self.paused
        if self.paused:
            self.water_timer.stop()
            self.stand_timer.stop()
            self.pause_item.title = "▶️ 继续提醒"
            self.title = "💤"
        else:
            # 重新计时（从现在起重新数间隔）
            self.water_timer.start()
            self.stand_timer.start()
            self.pause_item.title = "⏸ 暂停提醒"
            self.title = "💧"

    # ----- 立即测试 -----
    def test_now(self, _sender):
        notify("🔔 测试提醒", "看到这条就说明提醒能正常弹出来。", sound="Glass")

    # ----- 关于 -----
    def about(self, _sender):
        rumps.alert(
            "喝水 & 站立 提醒",
            "一个帮久坐上班族记得喝水、按时起身的小工具。\n\n"
            "· 点菜单里的间隔可随时修改\n"
            "· 设置会自动记住，下次启动仍然生效",
        )

    def quit_app(self, _sender):
        rumps.quit_application()


if __name__ == "__main__":
    ReminderApp().run()
