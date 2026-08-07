#!/usr/bin/env python3
"""Rasterise the brand SVGs into the raster formats the web still needs.

SVG is the source of truth for every mark in `brand/`. Two consumers cannot
read it, so those two get generated here rather than hand-exported:

  favicon.ico   old browsers, and the bare-domain request that ignores <link>
  og-card.png   Twitter, Slack and iMessage will not render an SVG preview
  apple-touch-icon.png   iOS ignores SVG for the home-screen icon

Run from the repo root:

    python3 scripts/build-brand-assets.py

It rewrites `website/static/img/`. Regenerate after any change under `brand/`;
a stale PNG beside a fresh SVG is the version people actually see, because the
raster is what the crawlers fetch.

Nothing here is needed to *build* mecha, so the dependencies are not installed
on this machine and do not need to be. Run it through uv instead:

    uv run --with cairosvg --with pillow python scripts/build-brand-assets.py

Needs `cairosvg` (plus libcairo2), `pillow`, and, for the og-card,
Inter and JetBrains Mono installed where fontconfig can find them. Both are
OFL; `--check-fonts` reports whether the real faces resolved, because cairosvg
silently substitutes and a substituted social card is off-brand in the one
place it is hardest to notice.
"""

import argparse
import io
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BRAND = ROOT / "brand"
OUT = ROOT / "website" / "static" / "img"

# The .ico carries three sizes: 16 is the tab, 32 is the bookmark bar and the
# retina tab, 48 is Windows' shortcut. Anything larger belongs in the SVG.
ICO_SIZES = (16, 32, 48)

REQUIRED_FONTS = ("Inter", "JetBrains Mono")

# The vectors the site actually serves: the navbar/hero pair, the footer's
# currentColor mark, and the modern favicon. See `main` for why the lockups
# are not in this list.
COPIED = ("logo.svg", "logo-light.svg", "logo-mono.svg", "favicon.svg")


def check_fonts() -> bool:
    """Report whether fontconfig resolves the brand faces to themselves."""
    ok = True
    for family in REQUIRED_FONTS:
        try:
            got = subprocess.run(
                ["fc-match", family], capture_output=True, text=True, check=True
            ).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            print(f"  ?  {family}: fc-match unavailable, cannot verify")
            continue
        # fc-match always answers; a substitution is the failure we care about.
        if family.split()[0].lower() in got.lower():
            print(f"  ok {family}: {got}")
        else:
            print(f"  !! {family}: substituted by {got}")
            ok = False
    return ok


def render(svg: pathlib.Path, png: pathlib.Path, width: int, height: int) -> None:
    import cairosvg

    png.parent.mkdir(parents=True, exist_ok=True)
    cairosvg.svg2png(
        url=str(svg), write_to=str(png), output_width=width, output_height=height
    )
    print(f"  {png.relative_to(ROOT)}  {width}x{height}")


def render_ico(svg: pathlib.Path, ico: pathlib.Path) -> None:
    import cairosvg
    from PIL import Image

    frames = []
    for size in ICO_SIZES:
        buf = cairosvg.svg2png(url=str(svg), output_width=size, output_height=size)
        frames.append(Image.open(io.BytesIO(buf)).convert("RGBA"))
    # Pillow writes every requested size into one .ico from the largest frame.
    frames[-1].save(ico, format="ICO", sizes=[(s, s) for s in ICO_SIZES])
    print(f"  {ico.relative_to(ROOT)}  {'/'.join(str(s) for s in ICO_SIZES)}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--check-fonts",
        action="store_true",
        help="verify Inter and JetBrains Mono resolve, and fail if they do not",
    )
    args = ap.parse_args()

    try:
        import cairosvg  # noqa: F401
    except ImportError:
        print(
            "cairosvg is not installed. Rather than installing it globally:\n"
            "  uv run --with cairosvg --with pillow python "
            "scripts/build-brand-assets.py",
            file=sys.stderr,
        )
        return 1

    print("fonts:")
    fonts_ok = check_fonts()
    if args.check_fonts and not fonts_ok:
        print(
            "\nA substituted face would ship in the social card. Install Inter and\n"
            "JetBrains Mono, or drop --check-fonts to accept the substitution.",
            file=sys.stderr,
        )
        return 1

    print("copying vector sources:")
    OUT.mkdir(parents=True, exist_ok=True)
    # Exactly what a page references, and nothing else. The lockups are not
    # here: the README points at `brand/` directly, so a second copy under
    # `static/` would be a file no page serves and no one regenerates.
    for name in COPIED:
        (OUT / name).write_bytes((BRAND / name).read_bytes())
        print(f"  {(OUT / name).relative_to(ROOT)}")

    # A file left behind by an earlier copy set outlives the reason it was
    # copied, and static/ has no index — so say so rather than let it sit.
    strays = sorted(
        p.name
        for p in OUT.glob("*.svg")
        if p.name not in COPIED and (BRAND / p.name).exists()
    )
    if strays:
        print(f"  note: {', '.join(strays)} are no longer copied; remove them")

    print("rasterising:")
    render(BRAND / "og-card.svg", OUT / "og-card.png", 1200, 630)
    render(BRAND / "apple-touch-icon.svg", OUT / "apple-touch-icon.png", 180, 180)
    render_ico(BRAND / "favicon.svg", OUT / "favicon.ico")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
