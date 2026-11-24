package imagestudio

import (
	"fmt"
	"image"
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	core "github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/egoist/mygo/ui"
)

func readyState(models ...string) *State {
	s := NewState()
	s.CheapSignedIn = true
	for _, id := range models {
		s.CheapModels = append(s.CheapModels, cheapauth.Model{ID: id})
	}
	return s
}

func refPaths(n int) []string {
	out := make([]string, n)
	for i := range out {
		out[i] = fmt.Sprintf("/refs/%02d.png", i)
	}
	return out
}

func TestReferencesRespectModelLimit(t *testing.T) {
	s := readyState("gpt-image-2", "grok-2-image")
	s.Prompt = "a cat"
	if over := s.AddReferencePaths(append(refPaths(2), refPaths(1)...)); over || len(s.References) != 2 {
		t.Fatalf("duplicates not skipped: %v over=%v", s.References, over)
	}
	if over := s.AddReferencePaths(refPaths(core.MaxReferences + 1)); !over || len(s.References) != core.MaxReferences {
		t.Fatalf("limit not applied: %d refs, over=%v", len(s.References), over)
	}
	if !s.CanSubmit() {
		t.Fatal("a draft at the limit cannot submit")
	}
	// Switching to a model with a lower limit keeps the draft but blocks it.
	s.Model = "grok-2-image"
	if s.ReferenceLimit() != core.MaxGrokReferences || s.CanSubmit() {
		t.Fatalf("Grok limit %d, submit %v", s.ReferenceLimit(), s.CanSubmit())
	}
}

func TestRemovingFirstReferenceDropsMask(t *testing.T) {
	s := readyState("gpt-image-2")
	s.AddReferencePaths(refPaths(2))
	s.MaskStrokes = []core.MaskStroke{{Width: 10, Points: []core.MaskPoint{{X: 1, Y: 1}}}}
	s.RemoveReference(1)
	if !s.HasMask() {
		t.Fatal("removing the second reference dropped the first's mask")
	}
	s.RemoveReference(0)
	if s.HasMask() || len(s.References) != 0 {
		t.Fatalf("mask outlived its image: refs=%v strokes=%d", s.References, len(s.MaskStrokes))
	}
}

func TestApplyJobRestoresParametersAndInputs(t *testing.T) {
	s := readyState("gpt-image-2", "gpt-image-1")
	strokes := []core.MaskStroke{{Width: 20, Points: []core.MaskPoint{{X: 3, Y: 4}}}}
	job := core.Job{Spec: core.GenerateSpec{
		Channel:        core.ChannelCheapRouter,
		Model:          "gpt-image-1",
		Prompt:         "repaint the sky",
		Size:           "1536x1024",
		Quality:        "high",
		Count:          3,
		ReferencePaths: refPaths(2),
		MaskStrokes:    strokes,
	}}
	s.ApplyJob(job)
	if s.Prompt != "repaint the sky" || s.EffectiveModel() != "gpt-image-1" || s.EffectiveSize() != "1536x1024" ||
		s.EffectiveQuality() != "high" || s.EffectiveCount() != 3 {
		t.Fatalf("parameters not restored: %+v", s)
	}
	if !slices.Equal(s.References, refPaths(2)) || !s.HasMask() {
		t.Fatalf("inputs not restored: refs=%v mask=%v", s.References, s.HasMask())
	}
	// The draft owns its copies: editing them leaves the job alone.
	s.MaskStrokes[0].Width = 99
	s.References[0] = "changed"
	if job.Spec.ReferencePaths[0] == "changed" {
		t.Fatal("draft shares the job's reference list")
	}

	s.ApplyJob(core.Job{Spec: core.GenerateSpec{Prompt: "plain", Model: "gpt-image-2"}})
	if len(s.References) != 0 || s.HasMask() {
		t.Fatal("a text-to-image job left the previous inputs")
	}
}

func TestVisibleFiltersByOutcomeAndPrompt(t *testing.T) {
	s := NewState()
	s.Jobs = []core.Job{
		{ID: "a", Status: core.JobDone, Spec: core.GenerateSpec{Prompt: "Red fox"}},
		{ID: "b", Status: core.JobRunning, Spec: core.GenerateSpec{Prompt: "blue whale"}},
		{ID: "c", Status: core.JobFailed, Spec: core.GenerateSpec{Prompt: "red car"}},
		{ID: "d", Status: core.JobCancelled, Spec: core.GenerateSpec{Prompt: "green tree"}},
	}
	ids := func() []string {
		var out []string
		for _, i := range s.Visible() {
			out = append(out, s.Jobs[i].ID)
		}
		return out
	}
	if got := ids(); !slices.Equal(got, []string{"a", "b", "c", "d"}) || s.Filtered() {
		t.Fatalf("all = %v", got)
	}
	s.Query = " RED "
	if got := ids(); !slices.Equal(got, []string{"a", "c"}) {
		t.Fatalf("query = %v", got)
	}
	s.Filter = FilterFailed
	if got := ids(); !slices.Equal(got, []string{"c"}) {
		t.Fatalf("failed+query = %v", got)
	}
	s.Query = ""
	if got := ids(); !slices.Equal(got, []string{"c", "d"}) {
		t.Fatalf("failed = %v", got)
	}
	s.Filter = FilterRunning
	if got := ids(); !slices.Equal(got, []string{"b"}) {
		t.Fatalf("running = %v", got)
	}
}

func TestMaskEditorPaintsUndoesAndApplies(t *testing.T) {
	s := readyState("gpt-image-2")
	if s.BeginMask() {
		t.Fatal("mask editor opened without a reference")
	}
	s.AddReferencePaths(refPaths(1))
	if !s.BeginMask() || !s.Mask.Loading || s.Mask.Width != core.DefaultBrushWidth {
		t.Fatalf("editor = %+v", s.Mask)
	}
	s.MaskLoaded("/other.png", ui.NewBitmap(image.NewRGBA(image.Rect(0, 0, 4, 4))), 4, 4)
	if !s.Mask.Loading {
		t.Fatal("a stale decode reached the editor")
	}
	s.MaskLoaded(s.References[0], ui.NewBitmap(image.NewRGBA(image.Rect(0, 0, 200, 100))), 200, 100)
	if s.Mask.Loading || s.Mask.Failed {
		t.Fatalf("editor not loaded: %+v", s.Mask)
	}

	m := &s.Mask
	m.Press(core.MaskPoint{X: 10, Y: 10})
	m.Move(core.MaskPoint{X: 10.2, Y: 10.2}) // under a pixel: dropped
	m.Move(core.MaskPoint{X: 80, Y: 40})
	m.Release()
	if len(m.Strokes) != 1 || len(m.Strokes[0].Points) != 2 || m.Drawing() {
		t.Fatalf("stroke = %+v", m.Strokes)
	}
	if m.cov.AlphaAt(45, 25).A != 255 {
		t.Fatal("preview did not paint along the stroke")
	}
	m.Erase = true
	m.Press(core.MaskPoint{X: 45, Y: 25})
	m.Release()
	if m.cov.AlphaAt(45, 25).A != 0 {
		t.Fatal("eraser did not clear the preview")
	}
	m.Undo()
	if len(m.Strokes) != 1 || m.cov.AlphaAt(45, 25).A != 255 {
		t.Fatal("undo did not restore the erased area")
	}

	s.CloseMask()
	if s.HasMask() || s.Mask.Open || s.Mask.Width != core.DefaultBrushWidth {
		t.Fatalf("cancel changed the draft: mask=%v editor=%+v", s.HasMask(), s.Mask)
	}

	s.BeginMask()
	s.MaskLoaded(s.References[0], ui.NewBitmap(image.NewRGBA(image.Rect(0, 0, 200, 100))), 200, 100)
	s.Mask.Press(core.MaskPoint{X: 50, Y: 50})
	s.Mask.Release()
	s.ApplyMask()
	if !s.HasMask() || s.Mask.Open {
		t.Fatal("apply did not keep the mask")
	}
	// Reopening edits the kept strokes; clearing and applying removes the mask.
	s.BeginMask()
	if len(s.Mask.Strokes) != 1 {
		t.Fatalf("reopened editor has %d strokes", len(s.Mask.Strokes))
	}
	s.Mask.Clear()
	s.ApplyMask()
	if s.HasMask() || s.MaskStrokes != nil {
		t.Fatal("cleared mask kept")
	}
}

func TestMaskLoadFailure(t *testing.T) {
	s := readyState("gpt-image-2")
	s.AddReferencePaths(refPaths(1))
	s.BeginMask()
	s.MaskLoaded(s.References[0], nil, 0, 0)
	if !s.Mask.Failed || s.Mask.Loading {
		t.Fatalf("editor = %+v", s.Mask)
	}
	s.Mask.Press(core.MaskPoint{X: 1, Y: 1})
	if len(s.Mask.Strokes) != 0 {
		t.Fatal("painted over an image that failed to load")
	}
}
