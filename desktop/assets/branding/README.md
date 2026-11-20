# Application icons

## CheapRouter Windows & cross-platform icon

`cheaprouter.png` is the 1024×1024 RGBA brand source extracted from
`resources/AppIcon.icns` (`ic10`).

Regenerate the multi-size Windows `.ico` (16, 24, 32, 48, 64, 128, and 256 px
PNG payloads) from the `desktop/` module root:

```sh
go generate ./assets/branding
# or directly:
go run ./cmd/gen-win-icon \
  -src assets/branding/cheaprouter.png \
  -out assets/branding/cheaprouter.ico \
  -out ../resources/windows/AppIcon.ico
```

- `mygo.json` top-level `icon` points to `assets/branding/cheaprouter.png` so
  `mygo build` embeds all 7 sizes into `CheapRouter.exe` via a temporary
  `.syso` (`RT_ICON` entries and `RT_GROUP_ICON` ID 1).
- `mygo.json` `windows.icon` points to `assets/branding/cheaprouter.ico`, which
  `cmd/sailry-desktop/windows/package.sh` passes to Inno Setup (`SetupIconFile`).
  The pinned MyGo CLI has no Windows icon override; it still uses the top-level
  PNG for the executable. The legacy Rust package uses `resources/windows/AppIcon.ico`.
- `assets/branding/branding.go` embeds `cheaprouter.png` so runtime windows set
  `WM_SETICON` icons via `Window.SetIcon`, which chooses sizes based on the
  window's DPI when called. The process and Inno shortcuts share the `sh.waku`
  AppUserModelID so the shell can group them.

## Legacy Sailry assets

`sailry-windows.svg` keeps the existing ribbon geometry and colors from
`internal/desktop/exporter/assets/sailry-mark.svg`. Its centered square canvas
reduces the transparent padding so the ribbon fills 96% of the icon's height.
The shared `sailry.png` retains its original padding for Mobile.

macOS keeps its separate `macos.icon` override, `Sailry.icns`; do not regenerate
that approved asset from the Windows source.
