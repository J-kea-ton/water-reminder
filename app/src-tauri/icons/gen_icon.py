#!/usr/bin/env python3
# 手绘小水豚应用图标，输出 1024x1024 RGBA PNG。只用标准库（zlib/struct）。
import struct, zlib, math, sys

S = 1024
buf = bytearray(4 * S * S)  # RGBA

def put(x, y, r, g, b, a):
    if x < 0 or y < 0 or x >= S or y >= S:
        return
    i = 4 * (y * S + x)
    # alpha over 合成到已有像素
    dr, dg, db, da = buf[i], buf[i+1], buf[i+2], buf[i+3]
    af = a / 255.0
    buf[i]   = int(r * af + dr * (1 - af))
    buf[i+1] = int(g * af + dg * (1 - af))
    buf[i+2] = int(b * af + db * (1 - af))
    buf[i+3] = min(255, int(a + da * (1 - af)))

def rounded_rect(x0, y0, x1, y1, rad, col):
    r, g, b, a = col
    for y in range(int(y0), int(y1)):
        for x in range(int(x0), int(x1)):
            # 圆角判定
            cx = min(max(x, x0 + rad), x1 - rad)
            cy = min(max(y, y0 + rad), y1 - rad)
            d = math.hypot(x - cx, y - cy)
            if d <= rad:
                aa = 1.0
                if d > rad - 1.5:
                    aa = max(0.0, (rad - d) / 1.5)
                put(x, y, r, g, b, int(a * aa))

def ellipse(cx, cy, rx, ry, col):
    r, g, b, a = col
    for y in range(int(cy - ry - 1), int(cy + ry + 1)):
        for x in range(int(cx - rx - 1), int(cx + rx + 1)):
            v = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2
            if v <= 1.0:
                aa = 1.0
                edge = 1.0 - v
                if edge < 0.03:
                    aa = edge / 0.03
                put(x, y, r, g, b, int(a * max(0.0, min(1.0, aa))))

CREAM  = (251, 247, 240, 255)
BROWN  = (216, 180, 137, 255)
MOUTH  = (246, 233, 214, 255)
EYE    = (74, 59, 48, 255)
NOSE   = (61, 52, 46, 255)
POMELO = (246, 169, 59, 255)
LEAF   = (124, 179, 66, 255)
CORAL  = (249, 140, 110, 90)
WHITE  = (255, 255, 255, 235)

# 背景奶油圆角卡
rounded_rect(40, 40, 984, 984, 200, CREAM)
# 落影
ellipse(512, 820, 250, 40, (239, 225, 198, 120))
# 耳朵
ellipse(360, 430, 55, 55, BROWN)
ellipse(664, 430, 55, 55, BROWN)
# 身体
rounded_rect(300, 430, 724, 800, 150, BROWN)
# 嘴部奶白
ellipse(512, 700, 150, 95, MOUTH)
# 腮红
ellipse(390, 620, 42, 26, CORAL)
ellipse(634, 620, 42, 26, CORAL)
# 眼睛
ellipse(438, 560, 48, 60, EYE)
ellipse(586, 560, 48, 60, EYE)
ellipse(452, 540, 14, 14, WHITE)
ellipse(600, 540, 14, 14, WHITE)
# 鼻
ellipse(512, 685, 26, 20, NOSE)
# 柚子
ellipse(512, 360, 110, 110, POMELO)
# 叶子
rounded_rect(520, 250, 585, 285, 16, LEAF)

def chunk(typ, data):
    c = struct.pack(">I", len(data)) + typ + data
    c += struct.pack(">I", zlib.crc32(typ + data) & 0xffffffff)
    return c

raw = bytearray()
for y in range(S):
    raw.append(0)  # filter type 0
    raw += buf[4 * y * S: 4 * (y + 1) * S]

png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")

out = sys.argv[1] if len(sys.argv) > 1 else "source-1024.png"
with open(out, "wb") as f:
    f.write(png)
print("wrote", out)
