#!/usr/bin/env python3
"""Regenerate the pictures `eval/image-read/cases.jsonl` attaches.

The question those cases ask: handed a picture it can already see, does the
model read the text in it with `document_read` (the local OCR model) when the
text is dense — a page, a table, a receipt — and leave a picture with no text
alone? So the text-heavy pictures are made to look like phone photos of paper:
small print, a slight tilt, a shadow, a little blur and JPEG loss. The short
screenshots are crisp, and the controls have no text at all.

Everything here is fictional, as every fixture in this repo is. The outputs
are committed, so running the eval needs neither this script nor its fonts;
it is here so the pictures can be remade, deliberately, with a fixed seed.

    python3 eval/image-read/make_fixtures.py
"""

import random
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

OUT = Path(__file__).resolve().parent / "workspace" / "inbox"
FONTS = "/usr/share/fonts/truetype/dejavu/"
SERIF = FONTS + "DejaVuSerif.ttf"
SERIF_BOLD = FONTS + "DejaVuSerif-Bold.ttf"
SANS = FONTS + "DejaVuSans.ttf"
SANS_BOLD = FONTS + "DejaVuSans-Bold.ttf"
MONO = FONTS + "DejaVuSansMono.ttf"

rng = random.Random(20260930)


def font(path, size):
    return ImageFont.truetype(path, size)


def photograph(page, name, long_side=1400):
    """Paper, as a phone sees it: a tilt, a shadow across it, a little blur
    and JPEG loss, on a desk-coloured margin."""
    w, h = page.size
    desk = Image.new("RGB", (w + 160, h + 160), (118, 104, 88))
    desk.paste(page, (80, 80))
    tilted = desk.rotate(rng.uniform(-1.8, 1.8), resample=Image.BICUBIC, fillcolor=(118, 104, 88))
    shade = Image.new("L", tilted.size, 0)
    d = ImageDraw.Draw(shade)
    for x in range(tilted.size[0]):
        d.line([(x, 0), (x, tilted.size[1])], fill=int(70 * x / tilted.size[0]))
    dark = Image.new("RGB", tilted.size, (0, 0, 0))
    shaded = Image.composite(dark, tilted, shade)
    blurred = shaded.filter(ImageFilter.GaussianBlur(0.7))
    k = long_side / max(blurred.size)
    small = blurred.resize((int(blurred.size[0] * k), int(blurred.size[1] * k)), Image.LANCZOS)
    small.save(OUT / name, quality=72)


def letter(title, lines, name):
    page = Image.new("RGB", (1240, 1754), (250, 248, 242))
    d = ImageDraw.Draw(page)
    d.text((110, 110), title, font=font(SERIF_BOLD, 34), fill=(20, 20, 20))
    body = font(SERIF, 21)
    y = 190
    for line in lines:
        d.text((110, y), line, font=body, fill=(30, 30, 30))
        y += 33
    photograph(page, name)


def table(title, header, rows, name):
    cols = len(header)
    page = Image.new("RGB", (1240, 1754), (252, 252, 250))
    d = ImageDraw.Draw(page)
    d.text((100, 110), title, font=font(SANS_BOLD, 32), fill=(20, 20, 20))
    widths = [1040 // cols] * cols
    x0, y0, rh = 100, 200, 58
    cell = font(SANS, 22)
    head = font(SANS_BOLD, 22)
    for r, row in enumerate([header] + rows):
        y = y0 + r * rh
        x = x0
        for c, text in enumerate(row):
            d.rectangle([x, y, x + widths[c], y + rh], outline=(90, 90, 90), width=2)
            d.text((x + 12, y + 16), str(text), font=head if r == 0 else cell, fill=(25, 25, 25))
            x += widths[c]
    photograph(page, name)


def receipt(shop, items, tax_rate, name):
    page = Image.new("RGB", (620, 1500), (246, 246, 240))
    d = ImageDraw.Draw(page)
    mono = font(MONO, 20)
    d.text((40, 40), shop, font=font(MONO, 26), fill=(20, 20, 20))
    y = 110
    for label, price in items:
        d.text((40, y), f"{label:<24}{price:>8.2f}", font=mono, fill=(25, 25, 25))
        y += 34
    subtotal = round(sum(p for _, p in items), 2)
    tax = round(subtotal * tax_rate, 2)
    total = round(subtotal + tax, 2)
    y += 20
    for label, amount in [("SUBTOTAL", subtotal), (f"TAX {tax_rate * 100:.2f}%", tax), ("TOTAL", total)]:
        d.text((40, y), f"{label:<24}{amount:>8.2f}", font=mono, fill=(15, 15, 15))
        y += 36
    d.text((40, y + 30), "THANK YOU - NO RETURNS AFTER 30 DAYS", font=font(MONO, 16), fill=(60, 60, 60))
    photograph(page.crop((0, 0, 620, y + 90)), name, long_side=1300)
    return subtotal, tax, total


def screenshot(lines, name, transparent=False):
    img = Image.new("RGBA" if transparent else "RGB", (900, 70 + 44 * len(lines)),
                    (0, 0, 0, 0) if transparent else (30, 32, 36))
    d = ImageDraw.Draw(img)
    colour = (20, 20, 20) if transparent else (230, 230, 230)
    for i, line in enumerate(lines):
        d.text((30, 30 + 44 * i), line, font=font(MONO if not transparent else SANS, 26), fill=colour)
    img.save(OUT / name)


def scene_landscape(name):
    img = Image.new("RGB", (1200, 800), (140, 190, 235))
    d = ImageDraw.Draw(img)
    d.ellipse([900, 90, 1060, 250], fill=(250, 210, 60))
    d.polygon([(0, 800), (0, 520), (380, 360), (760, 540), (1200, 420), (1200, 800)], fill=(80, 150, 80))
    d.rectangle([250, 470, 275, 600], fill=(100, 70, 40))
    d.ellipse([200, 380, 330, 500], fill=(40, 110, 50))
    photograph(img, name, long_side=1200)


def scene_shapes(name):
    img = Image.new("RGB", (1200, 800), (245, 245, 245))
    d = ImageDraw.Draw(img)
    d.ellipse([120, 250, 420, 550], fill=(210, 40, 40))
    d.rectangle([480, 250, 780, 550], fill=(40, 80, 200))
    d.polygon([(860, 550), (1010, 250), (1160, 550)], fill=(40, 160, 70))
    img.save(OUT / name)


def filler(n, topic):
    words = ("the survey team noted that conditions along the transect were consistent with "
             "previous seasons and that no further action is required at this stage of the "
             "review while the committee considers the remaining items on its agenda").split()
    out = []
    for _ in range(n):
        rng.shuffle(words)
        out.append(" ".join(words[:12]).capitalize() + f" ({topic}).")
    return out


def main():
    OUT.mkdir(parents=True, exist_ok=True)

    letter("Coastal Access Permit - Renewal Notice", filler(9, "access") + [
        "Your renewed permit reference is CAP-2026-04417. It authorises vessel",
        "access to the northern kelp transects for survey purposes only, and",
        "expires on 31 March 2027 unless renewed before that date.",
    ] + filler(12, "conditions"), "permit-letter.jpg")
    letter("Laboratory 3B - Safety Notice", filler(8, "safety") + [
        "The maximum occupancy of Laboratory 3B is 14 persons at any time.",
        "Eyewash stations are checked every 7 days. For any incident, call the",
        "duty officer on extension x4719 before leaving the room.",
    ] + filler(13, "procedure"), "safety-notice.jpg")
    letter("Budget Memo - Holdfast Project, Year 2", filler(10, "budget") + [
        "Following review, the equipment line is reduced from $18,400 to",
        "$12,950, and a carry-forward of $3,275 is approved for Year 3.",
    ] + filler(12, "reporting"), "budget-memo.jpg")

    table("Holdfast counts by site", ["Site", "2025", "2026", "Temp (C)"], [
        ["Site 1", 164, 171, 11.9], ["Site 2", 203, 188, 12.4], ["Site 3", 97, 112, 11.1],
        ["Site 4", 187, 212, 11.5], ["Site 5", 141, 129, 12.8], ["Site 6", 258, 233, 10.7],
        ["Site 7", 76, 81, 12.1], ["Site 8", 119, 144, 11.3],
    ], "holdfast-table.jpg")
    table("Autumn enrolment", ["Course", "Section", "Enrolled", "Cap"], [
        ["PSYC 1", 1, 188, 200], ["PSYC 1", 2, 174, 200], ["PSYC 10", 1, 61, 65],
        ["PSYC 28", 1, 38, 40], ["PSYC 28", 2, 33, 40], ["PSYC 45", 1, 22, 25],
        ["PSYC 60", 1, 17, 18], ["PSYC 81", 1, 9, 12],
    ], "enrolment-table.jpg")
    table("Reagent inventory", ["Item", "Lot", "Qty", "Expiry"], [
        ["Agarose", "AG-5521", 4, "2027-02"], ["Ethanol 70%", "ET-0931", 12, "2026-12"],
        ["Sodium azide", "SA-7784", 2, "2026-11"], ["Tris buffer", "TB-3310", 6, "2027-05"],
        ["Glycerol", "GL-2206", 3, "2028-01"], ["PBS 10x", "PB-8812", 8, "2027-03"],
        ["DMSO", "DM-4450", 1, "2026-10"], ["Triton X-100", "TX-6127", 2, "2027-08"],
    ], "reagent-table.jpg")

    totals = {}
    totals["hardware-receipt.jpg"] = receipt("HARBOUR HARDWARE", [
        ("Rope 10m", 18.99), ("Carabiner x4", 23.96), ("Duct tape", 7.49), ("Zip ties 100", 5.99),
        ("Waterproof bag", 34.50), ("Marker pens", 6.25), ("Tarp 3x4", 29.00), ("Gloves", 12.75),
        ("Batteries AA 8", 11.40), ("Headlamp", 27.99), ("Bungee x6", 9.95), ("Sealant", 8.30),
    ], 0.0625, "hardware-receipt.jpg")
    totals["cafe-receipt.jpg"] = receipt("TIDEPOOL CAFE", [
        ("Flat white", 4.75), ("Oat latte", 5.25), ("Chai", 4.50), ("Scone", 3.95),
        ("Bagel + cc", 5.60), ("Soup of day", 7.80), ("Sandwich", 11.25), ("Salad", 10.40),
        ("Brownie", 3.60), ("Water", 2.00), ("Muffin", 3.75), ("Cookie x2", 4.40),
    ], 0.08, "cafe-receipt.jpg")
    totals["field-receipt.jpg"] = receipt("NORTHSHORE MARINE SUPPLY", [
        ("Quadrat frame", 42.00), ("Tape measure 50m", 31.50), ("Slate + pencil", 14.25),
        ("Dive log", 9.99), ("Mesh bag", 16.80), ("Calipers", 38.75), ("Tags x50", 12.00),
        ("Zip bags", 6.49), ("Sunscreen", 13.20), ("Snorkel", 24.95), ("Fins", 58.00),
        ("Mask defog", 7.95),
    ], 0.07, "field-receipt.jpg")

    screenshot(["$ ./run-survey --site 4", "error: process exited with code 137 (out of memory)"],
               "terminal-error.png")
    screenshot(["Calendar update", "Meeting moved to Thursday 3:40 PM, Room 214"],
               "calendar-note.png", transparent=True)

    scene_landscape("landscape.jpg")
    scene_shapes("shapes.png")

    for name, (subtotal, tax, total) in totals.items():
        print(f"{name}: subtotal {subtotal:.2f} tax {tax:.2f} total {total:.2f}")


if __name__ == "__main__":
    main()
