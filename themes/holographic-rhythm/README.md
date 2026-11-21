# Holographic Rhythm

A portable Sailry v1 theme with an original virtual idol on a holographic
night-city banner, neon line icons and translucent indigo surfaces.

Import this directory through Settings → Appearance → Import. Choose either
light or dark mode. Repository theme directories are not installed automatically.
The package uses Sailry's v1 theme hooks, including independent switch tracks.

Both modes include a continuous window wallpaper, a welcome banner whose
character rises above its top edge, four card icons (dark mode; light mode tints
Kit's vector icons), sidebar group icons, circuit textures, composer and message
ornaments, wordmark colors, tabs and global button colors. Send, Stop and New
Conversation use a cyan-to-pink gradient with Kit vector glyphs; the wordmark
shares that gradient. Standard feature-rail glyphs
retain their shapes and semantic control colors.

The wallpapers use `cover`; the small circuit texture repeats on the cards.
All assets are compressed PNGs. Background opacity is kept at full strength;
the existing window surface preferences control its visibility through panels.
Changing those preferences can change text contrast. The package checks cover
the default main-panel tint, card states and modal control colors.

Cyan and pink stars twinkle above the content and navigation panel tints, with
3–5 pixel cores and brighter cross-shaped glints. The banner's character drifts gently and petals and stars float past its frame, with slower holographic specks in conversations. Light mode
uses deeper colors and lower opacity for visibility. Settings → Appearance →
Theme animation controls these layers.

Run the shared checks from the Sailry repository, passing this directory (or an
absolute path to a theme in another repository):

```sh
python3 scripts/check-theme.py themes/holographic-rhythm
./target/debug/sailry-desktop --check-theme themes/holographic-rhythm
```

The first command audits explicit solid color contrast. The second uses an
existing Desktop binary's strict import validator without starting a window,
Node or profile. See [shared checks](../README.md#checking-packages) for PNG
decoding and wallpaper checks. No package-specific test suite is needed.
These checks do not establish native UI interaction acceptance.
Artwork provenance and generation prompts are
in [ARTWORK.md](ARTWORK.md). Theme and original generated artwork use Apache-2.0.
