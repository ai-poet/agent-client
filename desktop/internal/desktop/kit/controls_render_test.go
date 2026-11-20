package kit

import (
	"go/ast"
	"go/parser"
	"go/token"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// captionControls is every control that shows a caption of its own. A new
// button type belongs here: ButtonBase draws no text, so a control that sets
// only Label compiles, passes every behaviour test and still shows an empty
// pill on screen. The caption may sit in a helper in this same file: the
// check follows up to two calls, so extracting one does not trip it, while a
// helper moved to another file does.
var captionControls = []string{
	"Primary", "PrimaryBusy", "Secondary", "SecondaryBusy",
	"Ghost", "Danger", "DangerBusy", "Tag",
}

// TestControlsDrawTheirOwnCaption guards the empty-button bug: each control
// must build a single-line ui.Text of its own, because ButtonBase.Label only
// names the control for assistive technology and draws nothing. The rendered
// tree cannot tell the two apart (Tester.Find matches accessibility names
// too), and MyGo lays a caption out inside its control either way, so this
// reads controls.go as the source of truth.
func TestControlsDrawTheirOwnCaption(t *testing.T) {
	file, err := parser.ParseFile(token.NewFileSet(), "controls.go", nil, 0)
	if err != nil {
		t.Fatalf("parse controls.go: %v", err)
	}
	decls := map[string]*ast.FuncDecl{}
	for _, decl := range file.Decls {
		if fn, ok := decl.(*ast.FuncDecl); ok && fn.Recv == nil {
			decls[fn.Name.Name] = fn
		}
	}
	for _, name := range captionControls {
		fn, ok := decls[name]
		if !ok {
			t.Errorf("%s has no function in controls.go; its caption must be built there", name)
			continue
		}
		if !drawsCaption(fn.Body, decls, 2) {
			t.Errorf("%s builds no single-line ui.Text: ButtonBase.Label draws nothing, so it would show an empty control", name)
		}
	}
}

// drawsCaption reports whether body builds a ui.Text chained with SingleLine,
// closures and calls to helpers in decls included, down to depth calls.
func drawsCaption(body *ast.BlockStmt, decls map[string]*ast.FuncDecl, depth int) bool {
	found := false
	ast.Inspect(body, func(n ast.Node) bool {
		if found {
			return false
		}
		call, ok := n.(*ast.CallExpr)
		if !ok {
			return true
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "SingleLine" && isText(sel.X) {
			found = true
			return false
		}
		if depth > 0 {
			if id, ok := call.Fun.(*ast.Ident); ok {
				if fn := decls[id.Name]; fn != nil && drawsCaption(fn.Body, decls, depth-1) {
					found = true
					return false
				}
			}
		}
		return true
	})
	return found
}

// isText reports whether expr is ui.Text(...), however it is chained.
func isText(expr ast.Expr) bool {
	call, ok := expr.(*ast.CallExpr)
	if !ok {
		return false
	}
	if sel, ok := call.Fun.(*ast.SelectorExpr); ok {
		if pkg, ok := sel.X.(*ast.Ident); ok && pkg.Name == "ui" && sel.Sel.Name == "Text" {
			return true
		}
		return isText(sel.X)
	}
	return false
}

// TestCaptionsAppearOnce catches a control drawing its caption twice. Every
// kit button exposes its label once as the accessible name and draws it once,
// so a frame holds exactly two: a third means the caption was drawn twice,
// which is what a call site that still adds its own ui.Text shows.
func TestCaptionsAppearOnce(t *testing.T) {
	const label = "重试"
	for _, tc := range []struct {
		name  string
		build func(*ui.Context) ui.Element
	}{
		{"Primary", func(c *ui.Context) ui.Element { return Primary(c, label, false) }},
		{"PrimaryBusy idle", func(c *ui.Context) ui.Element { return PrimaryBusy(c, label, false, false) }},
		{"PrimaryBusy busy", func(c *ui.Context) ui.Element { return PrimaryBusy(c, label, true, true) }},
		{"Secondary", func(c *ui.Context) ui.Element { return Secondary(c, label) }},
		{"SecondaryBusy busy", func(c *ui.Context) ui.Element { return SecondaryBusy(c, label, true) }},
		{"Ghost", func(c *ui.Context) ui.Element { return Ghost(c, label, false) }},
		{"Danger", func(c *ui.Context) ui.Element { return Danger(c, label) }},
		{"DangerBusy idle", func(c *ui.Context) ui.Element { return DangerBusy(c, label, false) }},
		{"DangerBusy busy", func(c *ui.Context) ui.Element { return DangerBusy(c, label, true) }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			build := tc.build
			tt := ui.NewTester(func(c *ui.Context) {
				theme.Apply(c, "", theme.Light)
				ui.Column(c).Padding(8).Children(func() { build(c) })
			}, 320, 100)
			tt.Frame()
			count := 0
			for _, text := range tt.Texts() {
				if text == label {
					count++
				}
			}
			if count != 2 {
				t.Fatalf("%s draws its caption %d times: %q", tc.name, count, tt.Texts())
			}
		})
	}
}

// TestButtonInkDiffersFromFill catches "drawn but invisible": the caption's
// color must differ from the fill it sits on. This does not judge contrast,
// only that the palette did not resolve both to the same pixels.
func TestButtonInkDiffersFromFill(t *testing.T) {
	for _, mode := range []struct {
		name string
		mode theme.Mode
	}{{"light", theme.Light}, {"dark", theme.Dark}} {
		var p *theme.Palette
		tt := ui.NewTester(func(c *ui.Context) {
			theme.Apply(c, "", mode.mode)
			p = theme.Of(c)
		}, 40, 40)
		tt.Frame()
		for _, pair := range []struct {
			name, ink, fill string
			a, b            ui.Color
		}{
			{"Primary", "OnPrimary", "ButtonPrimary", p.OnPrimary, p.ButtonPrimary},
			{"Secondary", "ButtonSecondaryForeground", "ButtonSecondary", p.ButtonSecondaryForeground, p.ButtonSecondary},
			{"Ghost", "Foreground", "Background", p.Foreground, p.Background},
			{"Danger", "DangerForeground", "Danger", p.DangerForeground, p.Danger},
			{"Tag", "Primary", "Primary@12%", p.Primary, p.Background.Mix(p.Primary, 0.12)},
		} {
			if pair.a == pair.b || pair.a.A == 0 {
				t.Errorf("%s: %s %s resolves to the fill %s", mode.name, pair.name, pair.ink, pair.fill)
			}
		}
	}
}
