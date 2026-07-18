#!/bin/bash
# 双击这个文件就能启动"喝水站立提醒"。
# 启动后可以直接关掉弹出来的终端窗口，右上角的 💧 图标会继续运行。

cd "$(dirname "$0")" || exit 1
nohup /usr/bin/python3 main.py >/dev/null 2>&1 &

echo "已启动。看屏幕右上角，应该出现一个 💧 图标。"
echo "这个终端窗口可以直接关掉。"
sleep 1
