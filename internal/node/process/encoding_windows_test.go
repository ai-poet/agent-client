package process

import (
	"bytes"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestNativeOutputEncoding(t *testing.T) {
	t.Run("recent UTF16 boundaries survive exhausted retention", func(t *testing.T) {
		stream := newRetained(t.TempDir(), &outputBudget{}, true, true)
		defer stream.evict()
		value := append([]byte{0xff, 0xfe}, bytes.Repeat([]byte{'a', 0, 0x3d, 0xd8, 0x42, 0xde}, 12000)...)
		stream.append(value)
		stream.finish()
		preview := stream.preview()
		if preview["retained_bytes"] != 0 || preview["transcoded"] != true || preview["invalid_utf8"] != false || strings.Contains(preview["tail"].(string), "\ufffd") || !strings.HasSuffix(preview["tail"].(string), "a🙂") {
			t.Fatal("exhausted retention split recent encoded output", preview)
		}
	})
	for _, fixture := range []struct {
		name  string
		bytes []byte
		want  string
	}{{"UTF-16", []byte{0xff, 0xfe, 'a', 0, 0x2d, 0x4e}, "a中"}, {"OEM", []byte{'a', 0x82}, "aé"}, {"UTF-8 BOM", []byte{0xef, 0xbb, 0xbf, 'a'}, "a"}} {
		t.Run(fixture.name, func(t *testing.T) {
			stream := newRetained(t.TempDir(), &outputBudget{limit: streamLimit}, false, true)
			defer stream.evict()
			// Make the OEM fallback independent of the runner's console settings.
			pageCode := newCodePage(437)
			stream.capture.decoder.pageCode, stream.decoder.pageCode = pageCode, pageCode
			for _, value := range fixture.bytes {
				stream.append([]byte{value})
			}
			stream.finish()
			if capture := stream.snapshot(false); capture.Text != fixture.want || capture.InvalidUtf8 {
				t.Fatal(capture)
			}
			preview := stream.preview()
			if preview["transcoded"] != true || preview["total_bytes"] != uint64(len(fixture.bytes)) || len(preview) != 2 {
				t.Fatal("small transcoded output lost original byte accounting", preview)
			}
			if fixture.name == "UTF-16" {
				_, err := stream.page(protocol.ProcessInfo{}, protocol.ProcessOutputOutputStreamStdout, 3, 4)
				if fault, ok := err.(*protocol.Fault); !ok || fault.Code != protocol.ErrorCodeInvalidRequest {
					t.Fatalf("split UTF-16 offset: got %v, want invalid request", err)
				}
			}
		})
	}
}
