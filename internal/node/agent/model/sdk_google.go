package model

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"iter"
	"net/http"
	"net/url"
	"strings"

	"charm.land/fantasy"
	"github.com/MoeForge/moego/base64"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model/google"
	"github.com/google/uuid"
	"google.golang.org/genai"
)

func (model *sdkModel) streamGoogle(ctx context.Context, raw []byte, call fantasy.Call, emitter *sdkEmitter) {
	name := model.config.Model
	if name == "" || strings.ContainsAny(name, "?&") || strings.Contains(name, "..") {
		emitter.fail(fmt.Errorf("invalid Gemini model parameter"))
		return
	}
	headers := http.Header{}
	for key, value := range model.config.Headers {
		headers.Set(key, value)
	}
	for key, value := range call.Headers {
		headers.Set(key, value)
	}
	if call.UserAgent != "" {
		headers.Set("User-Agent", call.UserAgent)
	}
	if model.config.vertex != nil {
		client, err := genai.NewClient(ctx, &genai.ClientConfig{
			Backend: genai.BackendVertexAI, Project: model.config.Project, Location: model.config.Location,
			HTTPClient: model.client, HTTPOptions: genai.HTTPOptions{BaseURL: model.config.Endpoint + "/", Headers: headers},
		})
		if err != nil {
			emitter.fail(err)
			return
		}
		body, err := decodeSDK(string(raw))
		if err != nil {
			emitter.fail(err)
			return
		}
		// Sailry has already shaped the complete Gemini request, and owns
		// continuation and retry. The SDK supplies the Vertex resource path.
		attempts, continuation := int32(1), false
		options := &genai.GenerateContentConfig{AutomaticContinuation: &continuation, HTTPOptions: &genai.HTTPOptions{
			ExtrasRequestProvider: func(map[string]any) map[string]any { return body },
			RetryOptions:          &genai.HTTPRetryOptions{Attempts: &attempts},
		}}
		consumeGoogle(emitter, func(yield func(string, error) bool) {
			for chunk, err := range client.Models.GenerateContentStream(ctx, name, nil, options) {
				if err != nil {
					yield("", err)
					return
				}
				raw, err := json.Marshal(chunk)
				if !yield(string(raw), err) {
					return
				}
			}
		})
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
	request.Header = headers
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

	consumeGoogle(emitter, func(yield func(string, error) bool) {
		scanner := bufio.NewScanner(response.Body)
		scanner.Buffer(make([]byte, 1024), 256*1024*1024)
		for scanner.Scan() {
			if data, ok := strings.CutPrefix(scanner.Text(), "data:"); ok && !yield(data, nil) {
				return
			}
		}
		if err := scanner.Err(); err != nil {
			yield("", err)
		}
	})
}

func consumeGoogle(emitter *sdkEmitter, chunks iter.Seq2[string, error]) {
	terminal, tools := false, false
	finish := fantasy.FinishReasonStop
	for data, err := range chunks {
		if err != nil {
			emitter.fail(err)
			return
		}
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
	if !terminal {
		emitter.fail(io.ErrUnexpectedEOF)
		return
	}
	if tools {
		finish = fantasy.FinishReasonToolCalls
	}
	emitter.emit(fantasy.StreamPart{Type: fantasy.StreamPartTypeFinish, FinishReason: finish})
}
