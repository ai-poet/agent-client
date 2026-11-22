package icons

import (
	"archive/zip"
	"bytes"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/appresources"
	"github.com/egoist/mygo/ui"
)

// logoVariant identifies an AI Logo SVG variant. The archive and naming scheme
// come from github.com/yldm-tech/ai-logo/mygo v1.20.0 (MIT).
type logoVariant string

const (
	logoMono       logoVariant = "mono"
	logoColor      logoVariant = "color"
	logoText       logoVariant = "text"
	logoTextColor  logoVariant = "text-color"
	logoTextCn     logoVariant = "text-cn"
	logoBrand      logoVariant = "brand"
	logoBrandColor logoVariant = "brand-color"
)

var logoSuffixes = map[logoVariant]string{
	logoMono: "", logoColor: "-color", logoText: "-text", logoTextColor: "-text-color",
	logoTextCn: "-text-cn", logoBrand: "-brand", logoBrandColor: "-brand-color",
}

type logoArchive struct {
	files map[string]*zip.File
	err   error
}

// Read the archive during package initialization, before a frame can ask for a
// mark. Subsequent lookups and SVG parsing operate only on the open archive.
var brandArchive = loadLogoArchive()

func loadLogoArchive() logoArchive {
	raw, err := appresources.Read("ai-logo/icons.zip")
	if err != nil {
		return logoArchive{err: err}
	}
	reader, err := zip.NewReader(bytes.NewReader(raw), int64(len(raw)))
	if err != nil {
		return logoArchive{err: err}
	}
	archive := logoArchive{files: make(map[string]*zip.File, len(reader.File))}
	for _, file := range reader.File {
		archive.files[file.Name] = file
	}
	return archive
}

func parseLogo(slug string, variant logoVariant) (*ui.SVG, error) {
	slug = strings.ToLower(strings.TrimSpace(slug))
	if slug == "" || slug == "." || slug == ".." || strings.ContainsAny(slug, `/\`) || strings.Contains(slug, "..") {
		return nil, errors.New("ai-logo: invalid icon id")
	}
	suffix, ok := logoSuffixes[variant]
	if !ok {
		return nil, fmt.Errorf("ai-logo: unsupported SVG variant %q", variant)
	}
	if brandArchive.err != nil {
		return nil, brandArchive.err
	}
	name := slug + suffix + ".svg"
	file := brandArchive.files[name]
	if file == nil {
		return nil, fmt.Errorf("ai-logo: icon %q with variant %q does not exist", slug, variant)
	}
	reader, err := file.Open()
	if err != nil {
		return nil, err
	}
	raw, readErr := io.ReadAll(reader)
	closeErr := reader.Close()
	if readErr != nil {
		return nil, readErr
	}
	if closeErr != nil {
		return nil, closeErr
	}
	return ui.ParseSVG(raw)
}
