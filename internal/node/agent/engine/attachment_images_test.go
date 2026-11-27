package engine

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"math/rand/v2"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func smallAttachmentPNG(t *testing.T) []byte {
	t.Helper()
	var b bytes.Buffer
	if err := png.Encode(&b, image.NewGray(image.Rect(0, 0, 4, 4))); err != nil {
		t.Fatal(err)
	}
	return b.Bytes()
}

func TestImageHistoryModelSwitch(t *testing.T) {
	for _, current := range []bool{false, true} {
		for _, textAdapter := range []bool{false, true} {
			t.Run(fmt.Sprintf("current %v text adapter %v", current, textAdapter), func(t *testing.T) {
				id, mime := protocol.NewAttachmentID(), "image/png"
				pixels := smallAttachmentPNG(t)
				message := canonical.NewMessage(canonical.User,
					canonical.Text(0, "keep this instruction"),
					canonical.Block{Index: 1, Kind: "image", Data: &canonical.AssetContent{Asset: canonical.AssetRef{Digest: "original", Mime: mime, Size: uint64(len(pixels))}}},
					canonical.Block{Index: 2, Kind: "file", Data: &canonical.FileContent{File: canonical.FileRef{URI: attachmentPrefix + id.String(), Mime: &mime}}})
				records := []canonical.Record{{ID: "image-input", Kind: "message", Data: message}}
				before, _ := json.Marshal(records)
				provider := protocol.DefaultConversationProvider()
				provider.Api, provider.Authentication = protocol.ConversationModelApiAnthropic, protocol.AuthenticationApiKey
				provider.Models = []protocol.ConversationModel{{ID: "vision", Vision: true}, {ID: "text"}}
				if textAdapter {
					provider.Api = protocol.ConversationModelApiPlugin
					selection := protocol.DefaultConversationPresetsBinding()
					selection.Adapter = &protocol.PluginModelAdapterBinding{Adapter: "text", Capabilities: protocol.PluginModelAdapterCapabilities{Vision: false}}
					binding := &selection
					provider.Binding = &binding
				}
				r := &Runtime{
					attachment: func(context.Context, protocol.TurnID, protocol.AttachmentID) (protocol.Attachment, bool, error) {
						return protocol.Attachment{ID: id, Spec: protocol.AttachmentSpec{Name: "picture.png", MediaType: mime, Size: uint64(len(pixels))}}, current, nil
					},
					attachmentBytes: func(context.Context, protocol.Attachment) ([]byte, error) { return pixels, nil },
				}
				x := &execution{ctx: t.Context(), inv: &store.AgentInvocation{Provider: &provider}}
				for _, selected := range []string{"vision", "text", "vision"} {
					x.inv.Turn.Config.Model = selected
					request, assets, err := r.hydrateAttachments(x, records)
					if err != nil {
						t.Fatal(err)
					}
					images, references := 0, 0
					for _, block := range request[0].Message().Content {
						if block.Kind == "image" {
							images++
						}
						if text, ok := block.Data.(*canonical.TextContent); ok && strings.Contains(text.Text, id.String()) {
							references++
						}
					}
					wantImages, wantAssets := 0, 0
					if selected == "vision" && !textAdapter {
						wantImages, wantAssets = 2, 1
					}
					if images != wantImages || len(assets) != wantAssets || references != 1 || request[0].Message().Content[0].Data.(*canonical.TextContent).Text != "keep this instruction" {
						t.Fatalf("%s request = %d images, %d assets, %d references", selected, images, len(assets), references)
					}
					after, _ := json.Marshal(records)
					if string(after) != string(before) {
						t.Fatal("model switch changed canonical input")
					}
				}
			})
		}
	}
}

func TestOmittedImageAttachmentUsesCapturedInspection(t *testing.T) {
	for _, test := range []struct {
		name                   string
		vision, tools, inspect bool
		wantInspection         bool
	}{
		{name: "inline", vision: true, tools: true, inspect: true},
		{name: "assigned inspection", tools: true, inspect: true, wantInspection: true},
		{name: "no inspection route", tools: true},
		{name: "model cannot call tools", inspect: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			id, mime := protocol.NewAttachmentID(), "application/octet-stream"
			pixels := smallAttachmentPNG(t)
			file := canonical.FileRef{URI: attachmentPrefix + id.String(), Mime: &mime}
			message := canonical.NewMessage(canonical.User, canonical.Text(0, "Describe my image"),
				canonical.Block{Index: 1, Kind: "file", Data: &canonical.FileContent{File: file}})
			records := []canonical.Record{{ID: "image-input", Kind: "message", Data: message}}
			provider := protocol.DefaultConversationProvider()
			provider.Models = []protocol.ConversationModel{{ID: "fixture", Vision: test.vision, Tools: test.tools}}
			x := &execution{ctx: t.Context(), inv: &store.AgentInvocation{Provider: &provider}}
			x.inv.Turn.Config.Model = "fixture"
			if test.inspect {
				operation := protocol.ToolOperationInspectMedia
				inspection := Tool{Deferred: true, Binding: &store.ApprovalBinding{Operation: &operation}}
				inspection.Definition.Name = "inspect_image"
				x.tools = []Tool{inspection}
			}
			r := &Runtime{
				attachment: func(context.Context, protocol.TurnID, protocol.AttachmentID) (protocol.Attachment, bool, error) {
					return protocol.Attachment{ID: id, Spec: protocol.AttachmentSpec{Name: "photo.png", MediaType: mime, Size: uint64(len(pixels))}}, true, nil
				},
				attachmentBytes: func(context.Context, protocol.Attachment) ([]byte, error) { return pixels, nil },
			}
			request, assets, err := r.hydrateAttachments(x, records)
			if err != nil {
				t.Fatal(err)
			}
			blocks := request[0].Message().Content
			note := blocks[len(blocks)-1].Data.(*canonical.TextContent).Text
			if test.vision != (len(assets) == 1) || strings.Contains(note, "read_office") {
				t.Fatalf("image assets = %d, note = %q", len(assets), note)
			}
			if test.wantInspection {
				if !strings.Contains(note, `"attachment":"`+id.String()+`"`) || !strings.Contains(note, "select:inspect_image") {
					t.Fatalf("image inspection did not use the attachment ID: %q", note)
				}
			} else if strings.Contains(note, "inspect_image") || strings.Contains(note, "lacks image input") != !test.vision {
				t.Fatalf("image note overstates capabilities: %q", note)
			}
		})
	}
}

func TestUnpreparableImageAttachmentRemainsReferenceOnly(t *testing.T) {
	t.Parallel()
	// The PNG signature is detectable, but there is no image header to decode.
	raw := []byte("\x89PNG\r\n\x1a\n")
	id, mime := protocol.NewAttachmentID(), "image/png"
	reads := 0
	runtime := &Runtime{
		attachment: func(_ context.Context, _ protocol.TurnID, got protocol.AttachmentID) (protocol.Attachment, bool, error) {
			return protocol.Attachment{ID: got, Spec: protocol.AttachmentSpec{Name: "broken.png", MediaType: mime, Size: uint64(len(raw))}}, true, nil
		},
		attachmentBytes: func(context.Context, protocol.Attachment) ([]byte, error) {
			reads++
			return raw, nil
		},
	}
	uri := attachmentPrefix + id.String()
	file := &canonical.FileContent{File: canonical.FileRef{URI: uri, Mime: &mime}}
	records := []canonical.Record{{ID: "image", Kind: "message", Data: canonical.NewMessage(canonical.User,
		canonical.Block{Kind: "file", Data: file},
	)}}
	provider := protocol.DefaultConversationProvider()
	provider.Api, provider.Authentication = protocol.ConversationModelApiResponses, protocol.AuthenticationApiKey
	provider.Models = []protocol.ConversationModel{{ID: "vision", Vision: true}}
	x := &execution{ctx: t.Context(), inv: &store.AgentInvocation{Provider: &provider}}
	x.inv.Turn.Config.Model = "vision"
	for step := range 2 {
		projected, assets, err := runtime.hydrateAttachments(x, records)
		if err != nil || len(assets) != 0 {
			t.Fatalf("step %d: expected no inline image, got %d assets, err=%v", step, len(assets), err)
		}
		blocks := projected[0].Message().Content
		if len(blocks) != 1 || blocks[0].Kind != "text" {
			t.Fatalf("step %d: expected just the attachment reference, got %+v", step, blocks)
		}
		note := blocks[0].Data.(*canonical.TextContent).Text
		if !strings.Contains(note, `"id":"`+id.String()+`"`) || !strings.Contains(note, "could not be prepared") {
			t.Fatalf("step %d: missing reference or reason: %s", step, note)
		}
	}
	if reads != 1 || x.attachments[id].data != nil {
		t.Fatalf("unpreparable image was read %d times or its bytes were retained", reads)
	}
	if original := records[0].Message().Content[0]; original.Kind != "file" || original.Data.(*canonical.FileContent).File.URI != uri {
		t.Fatalf("canonical attachment was changed: %+v", original)
	}
}

func TestOversizedUserImageAttachmentIsPreparedBeforeModelRequest(t *testing.T) {
	t.Parallel()
	photo := image.NewNRGBA(image.Rect(0, 0, 2000, 1400))
	rng := rand.New(rand.NewPCG(11, 13))
	for y := 0; y < 1400; y++ {
		for x := 0; x < 2000; x++ {
			jitter := int(rng.IntN(9)) - 4
			photo.SetNRGBA(x, y, color.NRGBA{
				R: uint8(min(255, max(0, x*255/2000+jitter))),
				G: uint8(min(255, max(0, y*255/1400+jitter))),
				B: uint8(min(255, max(0, (x+y)*255/3400+jitter))),
				A: 255,
			})
		}
	}
	var raw bytes.Buffer
	if err := png.Encode(&raw, photo); err != nil {
		t.Fatal(err)
	}
	id := protocol.NewAttachmentID()
	declared := "image/png"
	reads := 0
	runtime := &Runtime{
		attachment: func(_ context.Context, _ protocol.TurnID, got protocol.AttachmentID) (protocol.Attachment, bool, error) {
			return protocol.Attachment{ID: got, Spec: protocol.AttachmentSpec{Name: "retina.png", MediaType: declared, Size: uint64(raw.Len())}}, true, nil
		},
		attachmentBytes: func(_ context.Context, _ protocol.Attachment) ([]byte, error) {
			reads++
			return raw.Bytes(), nil
		},
	}
	records := []canonical.Record{{ID: "img-turn", Kind: "message", Data: canonical.NewMessage(canonical.User,
		canonical.Text(0, "Inspect this screenshot"),
		canonical.Block{Index: 1, Kind: "file", Data: &canonical.FileContent{File: canonical.FileRef{URI: attachmentPrefix + id.String(), Mime: &declared}}},
	)}}
	provider := protocol.DefaultConversationProvider()
	provider.Api, provider.Authentication = protocol.ConversationModelApiResponses, protocol.AuthenticationApiKey
	provider.Models = []protocol.ConversationModel{{ID: "vision", Vision: true}}
	x := &execution{ctx: t.Context(), inv: &store.AgentInvocation{Provider: &provider}}
	x.inv.Turn.Config.Model = "vision"

	projected, assets, err := runtime.hydrateAttachments(x, records)
	if err != nil {
		t.Fatal(err)
	}
	if len(assets) != 1 {
		t.Fatalf("got %d image assets, want 1", len(assets))
	}
	var sentBytes []byte
	for _, data := range assets {
		sentBytes = data
	}
	var assetBlock *canonical.AssetContent
	for _, block := range projected[0].Message().Content {
		if value, ok := block.Data.(*canonical.AssetContent); ok {
			assetBlock = value
		}
	}
	if assetBlock == nil || assetBlock.Asset.Mime != "image/jpeg" || len(sentBytes) >= raw.Len()/2 {
		t.Fatalf("expected prepared compact JPEG for oversized user screenshot: block=%+v sent=%d raw=%d", assetBlock, len(sentBytes), raw.Len())
	}
	// Second step reuses cached prepared image without re-reading disk.
	if _, assets2, err := runtime.hydrateAttachments(x, records); err != nil || len(assets2) != 1 || reads != 1 {
		t.Fatalf("second step reads=%d assets=%d err=%v", reads, len(assets2), err)
	}
}
