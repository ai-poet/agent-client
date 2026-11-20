// Command gen-win-icon builds a multi-size Windows .ico (16, 24, 32, 48, 64,
// 128, 256 px PNG payloads) from a square PNG source image.
package main

import (
	"bytes"
	"encoding/binary"
	"flag"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"log"
	"os"
	"path/filepath"
	"strings"
)

var windowsIconSizes = []int{16, 24, 32, 48, 64, 128, 256}

type pathList []string

func (p *pathList) String() string { return strings.Join(*p, ",") }
func (p *pathList) Set(v string) error {
	for _, item := range strings.Split(v, ",") {
		if trimmed := strings.TrimSpace(item); trimmed != "" {
			*p = append(*p, trimmed)
		}
	}
	return nil
}

func main() {
	src := flag.String("src", "assets/branding/cheaprouter.png", "square source PNG (>= 256x256)")
	var outs pathList
	flag.Var(&outs, "out", "output .ico path (repeatable or comma-separated)")
	flag.Parse()
	if len(outs) == 0 {
		outs = pathList{"assets/branding/cheaprouter.ico"}
	}

	raw, err := os.ReadFile(*src)
	if err != nil {
		log.Fatalf("read %s: %v", *src, err)
	}
	ico, err := buildICO(raw)
	if err != nil {
		log.Fatalf("build ico from %s: %v", *src, err)
	}
	for _, out := range outs {
		if err := os.MkdirAll(filepath.Dir(out), 0o755); err != nil {
			log.Fatalf("mkdir %s: %v", out, err)
		}
		if err := os.WriteFile(out, ico, 0o644); err != nil {
			log.Fatalf("write %s: %v", out, err)
		}
		fmt.Printf("wrote %s (%d bytes, sizes %v)\n", out, len(ico), windowsIconSizes)
	}
}

func buildICO(src []byte) ([]byte, error) {
	img, err := png.Decode(bytes.NewReader(src))
	if err != nil {
		return nil, fmt.Errorf("decode png: %w", err)
	}
	b := img.Bounds()
	if b.Dx() != b.Dy() || b.Dx() < 256 {
		return nil, fmt.Errorf("source PNG must be square and >= 256x256, got %dx%d", b.Dx(), b.Dy())
	}

	enc := png.Encoder{CompressionLevel: png.BestCompression}
	payloads := make([][]byte, len(windowsIconSizes))
	for i, size := range windowsIconSizes {
		var buf bytes.Buffer
		if err := enc.Encode(&buf, resizeSquare(img, size)); err != nil {
			return nil, fmt.Errorf("encode %dx%d: %w", size, size, err)
		}
		payloads[i] = buf.Bytes()
	}

	var out bytes.Buffer
	_ = binary.Write(&out, binary.LittleEndian, [3]uint16{0, 1, uint16(len(windowsIconSizes))})
	offset := 6 + 16*len(windowsIconSizes)
	for i, size := range windowsIconSizes {
		dim := byte(size)
		if size >= 256 {
			dim = 0
		}
		out.Write([]byte{dim, dim, 0, 0})
		_ = binary.Write(&out, binary.LittleEndian, [2]uint16{1, 32})
		_ = binary.Write(&out, binary.LittleEndian, [2]uint32{uint32(len(payloads[i])), uint32(offset)})
		offset += len(payloads[i])
	}
	for _, payload := range payloads {
		out.Write(payload)
	}
	return out.Bytes(), nil
}

// resizeSquare scales a square image with premultiplied-alpha area averaging
// so transparent borders do not darken when shrinking to small taskbar sizes.
func resizeSquare(img image.Image, size int) *image.NRGBA {
	b := img.Bounds()
	w, h := b.Dx(), b.Dy()
	dst := image.NewNRGBA(image.Rect(0, 0, size, size))
	if w == 0 || h == 0 {
		return dst
	}
	sx, sy := float64(w)/float64(size), float64(h)/float64(size)
	at := func(x, y int) (float64, float64, float64, float64) {
		c := color.NRGBAModel.Convert(img.At(b.Min.X+min(max(x, 0), w-1), b.Min.Y+min(max(y, 0), h-1))).(color.NRGBA)
		a := float64(c.A)
		return float64(c.R) * a, float64(c.G) * a, float64(c.B) * a, a
	}
	for y := range size {
		for x := range size {
			var r, g, bl, a, n float64
			x0, x1 := int(float64(x)*sx), max(int(float64(x+1)*sx), int(float64(x)*sx)+1)
			y0, y1 := int(float64(y)*sy), max(int(float64(y+1)*sy), int(float64(y)*sy)+1)
			for yy := y0; yy < y1; yy++ {
				for xx := x0; xx < x1; xx++ {
					cr, cg, cb, ca := at(xx, yy)
					r, g, bl, a, n = r+cr, g+cg, bl+cb, a+ca, n+1
				}
			}
			if a == 0 {
				continue
			}
			dst.SetNRGBA(x, y, color.NRGBA{
				R: uint8(r/a + 0.5),
				G: uint8(g/a + 0.5),
				B: uint8(bl/a + 0.5),
				A: uint8(a/n + 0.5),
			})
		}
	}
	return dst
}
