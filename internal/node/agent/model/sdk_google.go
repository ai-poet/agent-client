package model

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"

	"charm.land/fantasy"
	"github.com/MoeForge/moego/base64"
	"github.com/ai-poet/agent-client/internal/node/agent/model/google"
	"github.com/google/uuid"
)

func (model *sdkModel) streamGoogle(ctx context.Context, raw []byte, call fantasy.Call, emitter *sdkEmitter) {
	name := model.config.Model
	if name == "" || strings.ContainsAny(name, "?&") || strings.Contains(name, "..") {
		emitter.fail(fmt.Errorf("invalid Gemini model parameter"))
		return
	}
	if !strings.HasPrefix(name, "models/") && !strings.HasPrefix(name, "tunedModels/") {
		name = "models/" + name
	}
	target, err := url.Parse(model.config.Endpoint)
	if err != nil {
		emitter.fail(err)
		return
	}
	target = target.JoinPath("v1beta", name+":streamGenerateContent")
	target.RawQuery = "alt=sse"
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, target.String(), bytes.NewReader(raw))
	if err != nil {
		emitter.fail(err)
		return
	}
	for key, value := range model.config.Headers {
		request.Header.Set(key, value)
	}
	for key, value := range call.Headers {
		request.Header.Set(key, value)
	}
	if call.UserAgent != "" {
		request.Header.Set("User-Agent", call.UserAgent)
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("x-goog-api-key", model.config.Key)
	response, err := model.client.Do(request)
	if err != nil {
		emitter.fail(err)
		return
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		emitter.fail(fmt.Errorf("Gemini returned HTTP %d", response.StatusCode))
		return
	}

	terminal, tools := false, false
	finish := fantasy.FinishReasonStop
	scanner := bufio.NewScanner(response.Body)
	scanner.Buffer(make([]byte, 1024), 256*1024*1024)
	for scanner.Scan() {
		line := scanner.Text()
		if !strings.HasPrefix(line, "data:") {
			continue
		}
		data := strings.TrimPrefix(line, "data:")
		value, err := decodeSDK(data)
		if err != nil {
			emitter.fail(err)
			return
		}
		if wireObject(value["error"]) != nil {
			emitter.facts.body = []byte(data)
			emitter.fail(fmt.Errorf("model stream rejected the request"))
			return
		}
		emitter.updateUsage(wireObject(value["usageMetadata"]), "google")
		for _, value := range wireArray(value["candidates"]) {
			candidate := wireObject(value)
			emitter.googleSources(candidate)
			for _, value := range wireArray(wireObject(candidate["content"])["parts"]) {
				part := wireObject(value)
				id := "text"
				if part["thought"] == true {
					id = "reasoning"
				}
				function := wireObject(part["functionCall"])
				if function != nil {
					id = wireString(function["id"])
					if id == "" {
						id = uuid.NewString()
					}
				}
				metadata := fantasy.ProviderMetadata{}
				if signature, ok := part["thoughtSignature"].(string); ok {
					decoded, err := base64.StdEncoding.DecodeString(signature)
					if err != nil {
						emitter.fail(err)
						return
					}
					toolID := ""
					if function != nil {
						toolID = id
					}
					metadata[google.Name] = &google.ReasoningMetadata{Signature: string(decoded), ToolID: toolID}
					if function == nil && !emitter.emit(fantasy.StreamPart{Type: fantasy.StreamPartTypeReasoningEnd, ID: id, ProviderMetadata: metadata}) {
						return
					}
				}
				if text, ok := part["text"].(string); ok && text != "" {
					kind := fantasy.StreamPartTypeTextDelta
					if part["thought"] == true {
						kind = fantasy.StreamPartTypeReasoningDelta
					}
					if !emitter.emit(fantasy.StreamPart{Type: kind, ID: id, Delta: text}) {
						return
					}
				}
				if function != nil {
					arguments, err := json.Marshal(function["args"])
					if err != nil {
						emitter.fail(err)
						return
					}
					tools = true
					if !emitter.emit(fantasy.StreamPart{Type: fantasy.StreamPartTypeToolCall, ID: id, ToolCallName: wireString(function["name"]), ToolCallInput: string(arguments), ProviderMetadata: metadata}) {
						return
					}
				}
			}
			if reason := wireString(candidate["finishReason"]); reason != "" {
				terminal = true
				switch reason {
				case "STOP":
					finish = fantasy.FinishReasonStop
				case "MAX_TOKENS":
					finish = fantasy.FinishReasonLength
				case "SAFETY", "RECITATION", "BLOCKLIST", "PROHIBITED_CONTENT", "SPII":
					finish = fantasy.FinishReasonContentFilter
				default:
					finish = fantasy.FinishReasonUnknown
				}
			}
		}
	}
	if err := scanner.Err(); err != nil {
		emitter.fail(err)
		return
	}
	if !terminal {
		emitter.fail(io.ErrUnexpectedEOF)
		return
	}
	if tools {
		finish = fantasy.FinishReasonToolCalls
	}
	emitter.emit(fantasy.StreamPart{Type: fantasy.StreamPartTypeFinish, FinishReason: finish})
}
