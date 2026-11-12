# Writing skins

A skin is a TOML file that decides how a banner looks: colors, fonts, sizes
and optional extras such as a percent pill or a sparkline. Skins don't decide
where banners go or what they show, so any skin works on any banner.

## Quick start

1. Make a folder for your skins, for example `~/stocktape-skins`.
2. Create `my-skin.toml` in it:

   ```toml
   schema = 1
   name = "My Skin"
   extends = "classic-dark"

   [tokens]
   navy = "#1B1035"     # every color using $navy follows
   green = "#22C55E"
   ```

3. Run it, with live reload while you edit:

   ```sh
   stocktape --skins-dir ~/stocktape-skins --skin my-skin
   ```

   Save the file and the banner restyles. If the file has a mistake, the
   error is printed (with the file and key) and the previous look stays.

4. Or render a PNG without opening a window:

   ```sh
   stocktape --skins-dir ~/stocktape-skins --skin my-skin --preview my-skin.png --width 1280
   ```

`stocktape --list-skins` shows every skin, where it comes from, and any errors.

## How skins combine

- **`extends`** starts from another skin; your file only needs what changes.
  Chains are allowed (`a` extends `b` extends `classic-dark`).
- **Tokens** are named colors in `[tokens]`. Anywhere a color is expected you
  can write `"$name"`. Tokens are resolved after merging, so redefining a
  parent's token restyles everything that uses it.
- **`[text]`** holds defaults for every text element; `[symbol]`, `[price]`,
  `[change]` and `[percent]` override them.
- A skin in your folder with the same name as a built-in one replaces it.

## Reference

`crates/skin/builtin/classic-dark.toml` sets every value, with comments. The
other built-ins (`daylight`, `terminal`, `glass`) are short examples of
`extends`.

| Section | Keys |
|---|---|
| top level | `schema` (must be `1`), `name`, `description`, `extends` |
| `[tokens]` | any name = color |
| `[banner]` | `height`, `background`, `padding`, `spacing`, `baseline` (0–1) |
| `[text]`, `[symbol]`, `[price]` | `family`, `size`, `weight`, `color` |
| `[change]` | the text keys plus `show`, `arrow` |
| `[percent]` | the text keys plus `show` |
| `[colors]` | `up`, `down`, `flat` |
| `[separator]` | `color`, `width` (0 for none) |
| `[badge]` | `enabled`, `up_bg`, `up_fg`, `down_bg`, `down_fg`, `flat_bg`, `flat_fg`, `radius`, `padding_x`, `padding_y` |
| `[sparkline]` | `enabled`, `width`, `height`, `stroke` |

- **Colors:** `#RGB`, `#RRGGBB` or `#RRGGBBAA` (alpha last).
- **Sizes:** logical pixels; they scale with the display.
- **Fonts:** the bundled `Inter` and `JetBrains Mono` (weights 400 and 700;
  Inter also 800). Only bundled fonts are used, so a skin looks the same on
  every machine.
- **Unknown keys are errors**, so a typo like `heigth` is caught instead of
  silently doing nothing.

## Rules for shared skins

Skins may be inspired by business-TV tickers, but must not use any network's
name, logo or trademark. Describe them neutrally ("Classic Dark", not after a
channel).
