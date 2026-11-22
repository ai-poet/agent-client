// Package branding embeds the CheapRouter application icon assets.
package branding

import _ "embed"

//go:generate go run ../../cmd/gen-win-icon -src cheaprouter.png -out cheaprouter.ico
//go:generate python3 gen_mac_icon.py

// AppIconPNG is the square CheapRouter brand PNG used to set runtime window
// icons (WM_SETICON / taskbar icon) so windows match the embedded .exe icon.
//
//go:embed cheaprouter.png
var AppIconPNG []byte
