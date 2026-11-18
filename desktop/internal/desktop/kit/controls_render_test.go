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
// pill on screen.
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
		if !drawsCaption(fn.Body) {
			t.Errorf("%s builds no single-line ui.Text: ButtonBase.Label draws nothing, so it would show an empty control", name)
		}
	}
}

// drawsCaption reports whether body builds a ui.Text chained with SingleLine,
// closures included.
func drawsCaption(body *ast.BlockStmt) bool {
	found := false
	ast.Inspect(body, func(n ast.Node) bool {
		call, ok := n.(*ast.CallExpr)
		if !ok {
			return true
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "SingleLine" && isText(sel.X) {
			found = true
		}
		return !found
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
