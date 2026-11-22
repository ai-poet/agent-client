# Application icons

## CheapRouter desktop icons

`cheaprouter.png` is the 1024×1024 RGBA brand source extracted from
`resources/AppIcon.icns` (`ic10`).

Regenerate the multi-size Windows `.ico` and macOS `.icns` (16 through 1024 px,
including Retina entries) from the repository root with Go and Python 3 with
Pillow installed. No macOS SDK is needed:

```sh
go generate ./assets/branding
```

- `mygo.json` `macos.icon` selects `cheaprouter.icns`. MyGo copies it to
  `Contents/Resources/AppIcon.icns` and sets `CFBundleIconFile` to `AppIcon.icns`.
- `mygo.json` top-level `icon` points to `assets/branding/cheaprouter.png` so
  `mygo build` embeds all 7 sizes into `CheapRouter.exe` via a temporary
  `.syso` (`RT_ICON` entries and `RT_GROUP_ICON` ID 1).
- `mygo.json` `windows.icon` points to `assets/branding/cheaprouter.ico`, which
  `cmd/sailry-desktop/windows/package.sh` passes to Inno Setup (`SetupIconFile`).
  The pinned MyGo CLI has no Windows icon override; it still uses the top-level
  PNG for the executable.
- `assets/branding/branding.go` embeds `cheaprouter.png` so runtime windows set
  `WM_SETICON` icons via `Window.SetIcon`, which chooses sizes based on the
  window's DPI when called. The process and Inno shortcuts share the `sh.waku`
  AppUserModelID so the shell can group them.

## Legacy Sailry assets

`sailry-windows.svg` keeps the existing ribbon geometry and colors from
`internal/desktop/exporter/assets/sailry-mark.svg`. Its centered square canvas
reduces the transparent padding so the ribbon fills 96% of the icon's height.
The shared `sailry.png` retains its original padding for Mobile.
