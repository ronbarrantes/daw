"""Draws the app icon (assets/icon.png and icon-512.png): a 4x4 grid of step
pads, some lit.

Run from the repo root with `python3 packaging/make_icon.py` (needs Pillow).
The PNG is committed, so builds don't need Python.
"""

from PIL import Image, ImageDraw

SIZE = 1024
# macOS icons leave a margin around the rounded square.
MARGIN = 100
RADIUS = 185
GRID = 4
GAP = 34
PAD_RADIUS = 34

BACKGROUND_TOP = (46, 46, 52)
BACKGROUND_BOTTOM = (24, 24, 28)
OFF = (70, 70, 78)
DRUM = (255, 140, 40)
NOTE = (80, 180, 255)

# Which pads are lit: a beat in orange, a little melody in blue.
LIT = {
    (0, 0): DRUM, (2, 0): DRUM, (1, 1): DRUM, (3, 1): DRUM,
    (0, 2): DRUM, (2, 2): DRUM,
    (1, 2): NOTE, (3, 2): NOTE, (0, 3): NOTE, (2, 3): NOTE, (3, 3): NOTE,
}


def main():
    # Draw at 2x and scale down for smooth edges.
    scale = 2
    s = SIZE * scale
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    # Background: a vertical gradient clipped to a rounded square.
    gradient = Image.new("RGBA", (s, s))
    top, bottom = BACKGROUND_TOP, BACKGROUND_BOTTOM
    for y in range(s):
        t = y / (s - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        ImageDraw.Draw(gradient).line([(0, y), (s, y)], fill=color + (255,))
    mask = Image.new("L", (s, s), 0)
    box = [MARGIN * scale, MARGIN * scale, s - MARGIN * scale, s - MARGIN * scale]
    ImageDraw.Draw(mask).rounded_rectangle(box, RADIUS * scale, fill=255)
    img.paste(gradient, (0, 0), mask)

    draw = ImageDraw.Draw(img)
    inner = (SIZE - 2 * MARGIN) * scale
    inset = 110 * scale
    area = inner - 2 * inset
    pad = (area - (GRID - 1) * GAP * scale) / GRID
    origin = MARGIN * scale + inset
    for row in range(GRID):
        for col in range(GRID):
            x = origin + col * (pad + GAP * scale)
            y = origin + row * (pad + GAP * scale)
            color = LIT.get((col, row), OFF)
            draw.rounded_rectangle(
                [x, y, x + pad, y + pad], PAD_RADIUS * scale, fill=color + (255,)
            )

    img.resize((SIZE, SIZE), Image.LANCZOS).save("assets/icon.png")
    # Linux icon themes want exact sizes.
    img.resize((512, 512), Image.LANCZOS).save("assets/icon-512.png")


if __name__ == "__main__":
    main()
