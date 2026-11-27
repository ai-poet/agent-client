package conversation

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
)

func TestConversationSearchUnicodeHighlight(t *testing.T) {
	// A folded query can be longer than its entire source in bytes.
	if got := Mark("k", "K"); got != "<mark>k</mark>" {
		t.Errorf("Mark = %q", got)
	}
	const src = "前K🙂ſ后"
	if got := Mark(src+" `ſ` [x](ſ)", "s"); got != "前K🙂<mark>ſ</mark>后 `ſ` [x](ſ)" {
		t.Errorf("Mark = %q", got)
	}
	if got := Find([]Turn{{User: src}}, "s"); len(got) != 1 || !got[0].User {
		t.Errorf("Find = %+v", got)
	}
	got := spans(src, "k🙂", 0, &theme.Palette{})
	if len(got) != 3 || got[0].Text != "前" || got[1].Text != "K🙂" || got[2].Text != "ſ后" {
		t.Errorf("spans = %+v", got)
	}
}
