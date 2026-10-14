"""Draws the SnapForge icon (a capture frame with a spark) and writes icon.png, icon.ico and icon64.rgba.

    python scripts/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "crates" / "app" / "assets"
S = 1024  # drawn large, then downsampled for smooth edges


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((32, 32, S - 32, S - 32), radius=220, fill=(28, 30, 40, 255))
    # gradient sheen
    sheen = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sheen)
    for y in range(S):
        a = int(70 * (1 - y / S))
        sd.line((0, y, S, y), fill=(90, 140, 255, a))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((32, 32, S - 32, S - 32), radius=220, fill=255)
    img.paste(sheen, (0, 0), mask)

    accent = (96, 165, 250, 255)
    w, L, m = 56, 210, 190  # stroke, corner length, margin
    for (cx, cy, sx, sy) in ((m, m, 1, 1), (S - m, m, -1, 1), (m, S - m, 1, -1), (S - m, S - m, -1, -1)):
        d.line((cx, cy, cx + sx * L, cy), fill=accent, width=w)
        d.line((cx, cy, cx, cy + sy * L), fill=accent, width=w)
        d.ellipse((cx - w // 2, cy - w // 2, cx + w // 2, cy + w // 2), fill=accent)
        d.ellipse((cx + sx * L - w // 2, cy - w // 2, cx + sx * L + w // 2, cy + w // 2), fill=accent)
        d.ellipse((cx - w // 2, cy + sy * L - w // 2, cx + w // 2, cy + sy * L + w // 2), fill=accent)

    # four-point spark in the middle
    c, r, t = S // 2, 190, 34
    spark = [(c, c - r), (c + t, c - t), (c + r, c), (c + t, c + t), (c, c + r), (c - t, c + t), (c - r, c), (c - t, c - t)]
    d.polygon(spark, fill=(255, 214, 102, 255))
    return img


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    big = draw()
    big.resize((256, 256), Image.LANCZOS).save(OUT / "icon.png")
    big.save(OUT / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    (OUT / "icon64.rgba").write_bytes(big.resize((64, 64), Image.LANCZOS).tobytes())
    print("wrote", OUT)


if __name__ == "__main__":
    main()
