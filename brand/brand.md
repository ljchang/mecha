# mecha brand

The mark is 9A. An M whose outer strokes are two armour legs and whose vertex is
a notch cut into a heavy bar. A single slot sits between the legs; the legs
break at a knee and taper inward to two points on the ground. Same geometry at
every size and in the terminal.

## Geometry

Frame 63 × 54. Everything derives from one angle and one gap.

| Part | Value |
| --- | --- |
| Bar | y0–16, notch x24–39 cut 8.5 deep to a point at (31.5, 8.5) |
| Notch angle | 7.5 across per 8.5 down — the only slope in the mark |
| Gap | 4 units, used twice: bar to leg (y16–20) and at the knee (y35–39) |
| Upper leg | y20–35, x0–14 and x49–63 |
| Lower leg | y39–54, tapering inward at the notch angle to x27.24 / x35.76 |
| Slot | x21–42, y24–31 |

The feet stop 8.5 apart, which is the notch's own depth. The wedge of space
between them is a second V, pointing the same way as the one in the bar.

## Files

| File | Use |
| --- | --- |
| `logo.svg` | The mark. 63 × 54, accent-400 fill. Navbar, docs, anywhere ≥ 24px. |
| `logo-light.svg` | The same mark in accent-700, for a light ground. |
| `logo-mono.svg` | Same paths, `fill="currentColor"` — inherits the surrounding text colour. |
| `logo-lockup.svg` | Mark + wordmark + descriptor. README, presentations, footers. |
| `logo-lockup-light.svg` | The same lockup on a light ground: mark in accent-700, wordmark in void. GitHub's light theme, slides on white. |
| `favicon.svg` | 16px build. One deliberate deviation, below. |
| `apple-touch-icon.svg` | 180 × 180 on the void ground. |
| `og-card.svg` | 1200 × 630 social card. Rasterise to PNG before shipping. |
| `splash.rs` | The block mark for the TUI, with the slot carrying run state. |
| `banner.md` | README header, image or code-fence version. |
| `contact-sheet.html` | Every file rendered at real sizes. Open it to check a change. |

The Docusaurus theme is **not** here. It lives at `website/src/css/custom.css`
and is edited in place — a second copy under `brand/` would be the stale-asset
problem this file exists to prevent, and the Infima overrides only mean anything
against the version of Docusaurus that is installed. What has to agree with this
file is the token block at the top of it.

### What the favicon changes, and why

A 16px favicon has 16 pixels to spend, so one thing in the full mark cannot
survive and is redrawn rather than left to the rasteriser: **the feet are
blunted.** The full taper reaches x27.24 of 63; the favicon stops at x28 of 64,
keeping a 2px gap between the tips.

Everything else is snapped to a 4-unit grid (= 1px at 16px), so no edge in the
favicon lands mid-pixel. The notch keeps its point, because the bar has material
below it.

Three things still need a raster step, which SVG can't do: `favicon.ico` (for
old browsers, and for the bare `/favicon.ico` request that ignores the `<link>`
entirely), `og-card.png` (Twitter and Slack won't render SVG), and
`apple-touch-icon.png` (iOS ignores SVG for the home-screen icon).

## Install

This is wired up already; what follows is the record of how, so a change here
lands there.

```bash
uv run --with cairosvg --with pillow \
  python scripts/build-brand-assets.py --check-fonts
```

(Through uv, because rasterising a logo is not a reason to install cairosvg on
the machine that builds mecha.)

copies the vector sources into `website/static/img/` and rasterises the three
that need it. Run it after any change under `brand/` — a stale PNG beside a
fresh SVG is the version people actually see, because the raster is what the
crawlers fetch.

`--check-fonts` fails when Inter or JetBrains Mono did not resolve. cairosvg
substitutes silently, and a substituted social card is off-brand in the one
place nobody looks at it: the link preview, rendered on someone else's machine.
Both faces are OFL — [Inter](https://github.com/rsms/inter/releases),
[JetBrains Mono](https://github.com/JetBrains/JetBrainsMono/releases).

| Generated | From |
| --- | --- |
| `website/static/img/logo.svg`, `logo-light.svg`, `logo-mono.svg`, `favicon.svg` | copied verbatim |
| `website/static/img/favicon.ico` | `favicon.svg` at 16 / 32 / 48 |
| `website/static/img/og-card.png` | `og-card.svg` at 1200 × 630 |
| `website/static/img/apple-touch-icon.png` | `apple-touch-icon.svg` at 180 |

The lockups are deliberately not copied. The README points at `brand/` directly
and no page on the site references one, so a copy under `static/` would be a
file nothing serves and nobody regenerates — the same stale-asset problem as a
PNG left beside a fresh SVG, one level up. The script names any it finds rather
than deleting them.

`website/src/css/custom.css` holds the tokens and the Docusaurus wiring. It is
edited in place rather than copied from here, because the Infima overrides only
make sense against the version of Docusaurus that is installed; the token block
at its top is the part that must match this file.

## Colour

| Token | Hex | Role |
| --- | --- | --- |
| void | `#12141f` | Page ground, footer, icon plates |
| bg | `#161826` | Panels, navbar |
| surface | `#232532` | Cards, armour fills |
| section | `#262a60` | The one saturated field: hero band |
| accent-400 | `#b5abfc` | The mark, links on dark, the lit slot |
| accent-500 | `#9184d9` | Base accent, focus ring |
| accent-700 | `#5d5294` | Panel lines, muted structure in the terminal |
| hazard | `#e8a24a` | Held sends, read-only, the one called-out rule |
| text | `#e9e9ed` | Body |
| text-muted | `#9a9aa8` | Secondary, labels |

Hazard amber never fills an area — lines, ticks and single characters only.
There is no second accent; contrast comes from the ramps, not from more hues.

## Type

Inter 500 for headings, Inter 400 for body, JetBrains Mono for code, labels,
kickers and anything a user would type. Never mono for prose.

The wordmark is lowercase `mecha` in Inter 500, tracking -0.02em, because that
is what you type. Uppercase wide-tracked MECHA is allowed only as a graphic in
the hero band and the README banner.

## Usage

Do

- Keep clear space equal to the bar's height (16 units) on all four sides.
- Below 24px use `favicon.svg`, not `logo.svg`.
- On a light ground swap the fill to accent-700 `#5d5294`.
- In the terminal, structure in accent-700 and the slot in accent-400, so the
  slot can turn hazard when a send is held.

Don't

- No gradient across the mark, no outer glow, no rotation, no outline.
- Don't fill an area with hazard amber.
- Don't set the wordmark in anything but Inter 500.
- Don't stretch the mark; the 7.5:8.5 angle is the proportion and it appears
  three times.
