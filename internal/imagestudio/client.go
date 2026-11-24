package imagestudio

import (
	"bufio"
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"net/textproto"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
	"unicode"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/egoist/mygo/netproxy"
)

const (
	pollNetworkRetries = 5
	maxSyncBodyBytes   = 64 << 20
)

// Client performs background HTTP image generation, editing, and output downloads.
// All methods honor ctx cancellation and redact secrets from errors.
type Client struct {
	httpClient     *http.Client
	downloadClient *http.Client
	pollInterval   time.Duration
	taskTimeout    time.Duration
}

// NewClient constructs an image generation client with sane timeouts.
func NewClient() *Client {
	transport := netproxy.Transport()
	return &Client{
		httpClient:     &http.Client{Transport: transport, Timeout: 16 * time.Minute},
		downloadClient: &http.Client{Transport: transport, Timeout: 5 * time.Minute},
		pollInterval:   PollInterval,
		taskTimeout:    TaskTimeout + 2*time.Minute,
	}
}

// WithTimings returns a shallow copy of Client using custom async poll interval
// and task timeout durations when positive.
func (c *Client) WithTimings(pollInterval, taskTimeout time.Duration) *Client {
	copy := *c
	if pollInterval > 0 {
		copy.pollInterval = pollInterval
	}
	if taskTimeout > 0 {
		copy.taskTimeout = taskTimeout
	}
	return &copy
}

func (c *Client) effectivePollInterval() time.Duration {
	if c.pollInterval > 0 {
		return c.pollInterval
	}
	return PollInterval
}

func (c *Client) effectiveTaskTimeout() time.Duration {
	if c.taskTimeout > 0 {
		return c.taskTimeout
	}
	return TaskTimeout + 2*time.Minute
}

// GenerateCheapRouter calls CheapRouter's Images API (generations or edits when
// spec.References is non-empty). It attempts async task submission + polling
// when tryAsync is true and falls back to synchronous generation (SSE stream
// for OpenAI image models, JSON for Grok/others) when async tasks are unavailable.
func (c *Client) GenerateCheapRouter(
	ctx context.Context,
	origin, apiKey string,
	spec GenerateSpec,
	tryAsync bool,
	onTaskID func(string),
) ([]ImageOutput, bool, *ImageError) {
	if spec.IsEdit() {
		if err := validateEdit(spec); err != nil {
			return nil, !tryAsync, err
		}
	}
	asyncDisabled := !tryAsync
	if tryAsync {
		taskID, err := c.submitCheapRouterAsync(ctx, origin, apiKey, spec)
		if err == nil {
			if onTaskID != nil {
				onTaskID(taskID)
			}
			outputs, pollErr := c.pollCheapRouterTask(ctx, origin, apiKey, taskID)
			return outputs, false, pollErr
		}
		if err.Kind == ErrCancelled {
			return nil, false, err
		}
		if err.Kind == ErrAsyncUnavailable {
			asyncDisabled = true
		} else if err.Kind != ErrWrongPlatform {
			return nil, false, err
		}
	}
	outputs, err := c.generateCheapRouterSync(ctx, origin, apiKey, spec)
	return outputs, asyncDisabled, err
}

// GenerateCustom calls a user-configured OpenAI Images-compatible endpoint
// (POST /v1/images/generations or POST /v1/images/edits when spec.References is
// non-empty), supporting both b64_json and url responses.
func (c *Client) GenerateCustom(ctx context.Context, cfg CustomConfig, spec GenerateSpec) ([]ImageOutput, *ImageError) {
	model := strings.TrimSpace(spec.Model)
	if model == "" {
		model = strings.TrimSpace(cfg.Model)
	}
	if model == "" {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "model is required"}
	}
	spec.Model = model
	extra, extraErr := ParseExtraParams(cfg.ExtraParams)
	if extraErr != nil {
		if ie, ok := extraErr.(*ImageError); ok {
			return nil, ie
		}
		return nil, &ImageError{Kind: ErrBadRequest, Message: "invalid extra parameters"}
	}
	if spec.IsEdit() {
		endpoint, err := EditsURL(cfg.BaseURL)
		if err != nil {
			if ie, ok := err.(*ImageError); ok {
				return nil, ie
			}
			return nil, &ImageError{Kind: ErrBadRequest, Message: "invalid base URL"}
		}
		outputs, editErr := c.GenerateEdits(ctx, endpoint, cfg.APIKey, spec, extra)
		if editErr != nil && (editErr.Status == http.StatusNotFound || editErr.Status == http.StatusMethodNotAllowed || editErr.Status == http.StatusNotImplemented) {
			return nil, &ImageError{
				Kind:    ErrBadRequest,
				Status:  editErr.Status,
				Message: fmt.Sprintf("custom image API does not support image editing (/v1/images/edits, HTTP %d): %s", editErr.Status, editErr.Message),
			}
		}
		return outputs, editErr
	}
	endpoint, err := GenerationsURL(cfg.BaseURL)
	if err != nil {
		if ie, ok := err.(*ImageError); ok {
			return nil, ie
		}
		return nil, &ImageError{Kind: ErrBadRequest, Message: "invalid base URL"}
	}
	payload := buildJSONPayload(model, spec.Prompt, spec.Size, spec.Quality, spec.ResponseFormat, spec.NormalizedCount(), false, false, extra)
	body, status, reqErr := c.postJSON(ctx, endpoint, cfg.APIKey, payload, 10*time.Minute)
	if reqErr != nil {
		return nil, reqErr
	}
	if status < 200 || status >= 300 {
		return nil, ClassifyError(status, body, cfg.APIKey)
	}
	outputs, parseErr := ParseOutputs(body, cfg.APIKey)
	if parseErr != nil {
		return nil, parseErr
	}
	return outputs, nil
}

// GenerateEdits sends a multipart/form-data request to an OpenAI-compatible
// POST /v1/images/edits endpoint with one or more reference images ("image"
// parts), supporting both b64_json and url responses.
func (c *Client) GenerateEdits(
	ctx context.Context,
	endpoint, apiKey string,
	spec GenerateSpec,
	extra map[string]any,
) ([]ImageOutput, *ImageError) {
	if err := validateEdit(spec); err != nil {
		return nil, err
	}
	if strings.TrimSpace(spec.Model) == "" {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "model is required"}
	}
	grok := cheapauth.IsGrokImageModel(spec.Model)
	payload, contentType, err := buildMultipartPayload(spec, spec.NormalizedCount(), false, grok, extra, "image")
	if err != nil {
		return nil, err
	}
	body, status, reqErr := c.postMultipart(ctx, endpoint, apiKey, contentType, payload, 10*time.Minute)
	if reqErr != nil {
		return nil, reqErr
	}
	if status < 200 || status >= 300 {
		return nil, ClassifyError(status, body, apiKey)
	}
	return ParseOutputs(body, apiKey)
}

func validateReferences(refs []ReferenceImage) *ImageError {
	if len(refs) == 0 {
		return &ImageError{Kind: ErrBadRequest, Message: "at least one reference image is required for editing"}
	}
	for i, ref := range refs {
		if len(ref.Data) == 0 {
			return &ImageError{Kind: ErrBadRequest, Message: fmt.Sprintf("reference image %d is empty", i+1)}
		}
		if len(ref.Data) > MaxReferenceBytes {
			return &ImageError{Kind: ErrBadRequest, Message: fmt.Sprintf("reference image %d exceeds the 20 MiB upload limit", i+1)}
		}
	}
	return nil
}

func validateEdit(spec GenerateSpec) *ImageError {
	if err := validateReferences(spec.References); err != nil {
		return err
	}
	if spec.Mask != nil && len(spec.Mask.Data) == 0 {
		return &ImageError{Kind: ErrBadRequest, Message: "mask image is empty"}
	}
	return nil
}

func (c *Client) submitCheapRouterAsync(ctx context.Context, origin, apiKey string, spec GenerateSpec) (string, *ImageError) {
	relPath := "images/generations/async"
	if spec.IsEdit() {
		relPath = "images/edits/async"
	}
	endpoint, err := CheapRouterImageURL(origin, relPath)
	if err != nil {
		return "", &ImageError{Kind: ErrBadRequest, Message: "invalid gateway endpoint"}
	}
	grok := cheapauth.IsGrokImageModel(spec.Model)
	var (
		body   []byte
		status int
		reqErr *ImageError
	)
	if spec.IsEdit() {
		payload, contentType, buildErr := buildMultipartPayload(spec, spec.NormalizedCount(), false, grok, nil, "image[]")
		if buildErr != nil {
			return "", buildErr
		}
		body, status, reqErr = c.postMultipart(ctx, endpoint, apiKey, contentType, payload, 3*time.Minute)
	} else {
		payload := buildJSONPayload(spec.Model, spec.Prompt, spec.Size, spec.Quality, spec.ResponseFormat, spec.NormalizedCount(), false, grok, nil)
		body, status, reqErr = c.postJSON(ctx, endpoint, apiKey, payload, 3*time.Minute)
	}
	if reqErr != nil {
		return "", reqErr
	}
	if status < 200 || status >= 300 {
		return "", ClassifyError(status, body, apiKey)
	}
	var obj map[string]any
	if err := json.Unmarshal(body, &obj); err != nil {
		return "", ClassifyError(status, body, apiKey)
	}
	if rawErr, exists := obj["error"]; exists && rawErr != nil {
		return "", ClassifyError(status, body, apiKey)
	}
	for _, field := range []string{"id", "task_id"} {
		if v, ok := obj[field].(string); ok && strings.TrimSpace(v) != "" {
			return strings.TrimSpace(v), nil
		}
	}
	return "", &ImageError{Kind: ErrOther, Message: "gateway returned no task id"}
}

func (c *Client) pollCheapRouterTask(ctx context.Context, origin, apiKey, taskID string) ([]ImageOutput, *ImageError) {
	endpoint, err := CheapRouterImageURL(origin, "images/tasks/"+url.PathEscape(strings.TrimSpace(taskID)))
	if err != nil {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "invalid task endpoint", TaskID: taskID}
	}
	withTask := func(ie *ImageError) *ImageError {
		if ie != nil && ie.TaskID == "" {
			ie.TaskID = taskID
		}
		return ie
	}
	deadline := time.Now().Add(c.effectiveTaskTimeout())
	networkFailures := 0
	ticker := time.NewTicker(c.effectivePollInterval())
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			if errors.Is(ctx.Err(), context.DeadlineExceeded) {
				return nil, &ImageError{Kind: ErrTimeout, Message: "image task timed out", TaskID: taskID}
			}
			return nil, &ImageError{Kind: ErrCancelled, Message: "cancelled", TaskID: taskID}
		case <-ticker.C:
		}
		if time.Now().After(deadline) {
			return nil, &ImageError{Kind: ErrTimeout, Message: "image task timed out", TaskID: taskID}
		}
		body, status, reqErr := c.getJSON(ctx, endpoint, apiKey, 30*time.Second)
		if reqErr != nil {
			if reqErr.Kind == ErrCancelled {
				return nil, withTask(reqErr)
			}
			if reqErr.Kind == ErrNetwork && networkFailures < pollNetworkRetries {
				networkFailures++
				if time.Now().After(deadline) {
					return nil, &ImageError{Kind: ErrTimeout, Message: "image task timed out", TaskID: taskID}
				}
				continue
			}
			return nil, withTask(reqErr)
		}
		if status == http.StatusNotFound {
			classified := ClassifyError(status, body, apiKey)
			classified.Kind = ErrTaskLost
			return nil, withTask(classified)
		}
		if status < 200 || status >= 300 {
			return nil, withTask(ClassifyError(status, body, apiKey))
		}
		networkFailures = 0
		done, outputs, taskErr := parseTaskResponse(body, apiKey)
		if taskErr != nil {
			return nil, withTask(taskErr)
		}
		if done {
			return outputs, nil
		}
		if time.Now().After(deadline) {
			return nil, &ImageError{Kind: ErrTimeout, Message: "image task timed out", TaskID: taskID}
		}
	}
}

func (c *Client) generateCheapRouterSync(ctx context.Context, origin, apiKey string, spec GenerateSpec) ([]ImageOutput, *ImageError) {
	relPath := "images/generations"
	if spec.IsEdit() {
		relPath = "images/edits"
	}
	endpoint, err := CheapRouterImageURL(origin, relPath)
	if err != nil {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "invalid gateway endpoint"}
	}
	grok := cheapauth.IsGrokImageModel(spec.Model)
	count := spec.NormalizedCount()
	sendOne := func(n int, stream bool) ([]byte, int, *ImageError) {
		if spec.IsEdit() {
			payload, contentType, buildErr := buildMultipartPayload(spec, n, stream, grok, nil, "image[]")
			if buildErr != nil {
				return nil, 0, buildErr
			}
			return c.postMultipart(ctx, endpoint, apiKey, contentType, payload, 16*time.Minute)
		}
		payload := buildJSONPayload(spec.Model, spec.Prompt, spec.Size, spec.Quality, spec.ResponseFormat, n, stream, grok, nil)
		return c.postJSON(ctx, endpoint, apiKey, payload, 16*time.Minute)
	}
	if grok {
		body, status, reqErr := sendOne(count, false)
		if reqErr != nil {
			return nil, reqErr
		}
		if status < 200 || status >= 300 {
			return nil, ClassifyError(status, body, apiKey)
		}
		return ParseOutputs(body, apiKey)
	}
	var outputs []ImageOutput
	for i := 0; i < count; i++ {
		if ctx.Err() != nil {
			if len(outputs) > 0 {
				return outputs, nil
			}
			return nil, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
		}
		body, status, reqErr := sendOne(1, true)
		if reqErr != nil {
			if len(outputs) == 0 {
				return nil, reqErr
			}
			break
		}
		if status < 200 || status >= 300 {
			if len(outputs) == 0 {
				return nil, ClassifyError(status, body, apiKey)
			}
			break
		}
		batch, parseErr := ParseOutputs(body, apiKey)
		if parseErr != nil {
			if len(outputs) == 0 {
				return nil, parseErr
			}
			break
		}
		outputs = append(outputs, batch...)
	}
	if len(outputs) == 0 {
		return nil, &ImageError{Kind: ErrOther, Message: "gateway returned no images"}
	}
	return outputs, nil
}

func buildJSONPayload(model, prompt, size, quality, responseFormat string, count int, stream, grok bool, extra map[string]any) map[string]any {
	body := make(map[string]any, 8+len(extra))
	for k, v := range extra {
		body[k] = v
	}
	body["model"] = strings.TrimSpace(model)
	body["prompt"] = prompt
	if count < 1 {
		count = 1
	}
	body["n"] = count
	if s := strings.TrimSpace(size); s != "" && !strings.EqualFold(s, "auto") {
		body["size"] = s
	} else if strings.EqualFold(strings.TrimSpace(size), "auto") && !grok {
		body["size"] = "auto"
	}
	if !grok {
		if q := strings.TrimSpace(quality); q != "" && !strings.EqualFold(q, "auto") {
			body["quality"] = q
		}
	}
	if stream {
		body["stream"] = true
	} else if rf := strings.TrimSpace(responseFormat); rf != "" {
		body["response_format"] = rf
	} else if grok {
		if _, set := body["response_format"]; !set {
			body["response_format"] = "b64_json"
		}
	}
	return body
}

func buildMultipartPayload(
	spec GenerateSpec,
	count int,
	stream, grok bool,
	extra map[string]any,
	imageField string,
) ([]byte, string, *ImageError) {
	if count < 1 {
		count = 1
	}
	var buf bytes.Buffer
	writer := multipart.NewWriter(&buf)

	written := map[string]bool{}
	writeField := func(name, value string) error {
		written[name] = true
		return writer.WriteField(name, value)
	}
	if err := writeField("model", strings.TrimSpace(spec.Model)); err != nil {
		return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart model"}
	}
	if err := writeField("prompt", spec.Prompt); err != nil {
		return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart prompt"}
	}
	if err := writeField("n", strconv.Itoa(count)); err != nil {
		return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart count"}
	}
	if s := strings.TrimSpace(spec.Size); s != "" && !strings.EqualFold(s, "auto") {
		if err := writeField("size", s); err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart size"}
		}
	} else if strings.EqualFold(strings.TrimSpace(spec.Size), "auto") && !grok {
		if err := writeField("size", "auto"); err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart size"}
		}
	}
	if !grok {
		if q := strings.TrimSpace(spec.Quality); q != "" && !strings.EqualFold(q, "auto") {
			if err := writeField("quality", q); err != nil {
				return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart quality"}
			}
		}
	}
	if stream {
		if err := writeField("stream", "true"); err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart stream"}
		}
	} else if rf := strings.TrimSpace(spec.ResponseFormat); rf != "" {
		if err := writeField("response_format", rf); err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart response_format"}
		}
	} else if grok {
		if _, set := extra["response_format"]; !set {
			if err := writeField("response_format", "b64_json"); err != nil {
				return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart response_format"}
			}
		}
	}
	for k, v := range extra {
		if written[k] || k == "image" || k == "image[]" || v == nil {
			continue
		}
		var text string
		switch val := v.(type) {
		case string:
			text = val
		case bool:
			text = strconv.FormatBool(val)
		case float64:
			text = strconv.FormatFloat(val, 'f', -1, 64)
		default:
			if encoded, err := json.Marshal(val); err == nil {
				text = string(encoded)
			}
		}
		if text != "" {
			if err := writer.WriteField(k, text); err != nil {
				return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to encode multipart extra parameter"}
			}
		}
	}
	for i, ref := range spec.References {
		ext := SniffExtension(ref.Data)
		mime := strings.TrimSpace(ref.MimeType)
		if mime == "" {
			mime = MimeTypeForExt(ext)
		}
		filename := sanitizeMultipartFilename(ref.Filename, fmt.Sprintf("reference-%d.%s", i+1, ext))
		header := make(textproto.MIMEHeader)
		header.Set("Content-Disposition", fmt.Sprintf(`form-data; name="%s"; filename="%s"`, imageField, filename))
		header.Set("Content-Type", mime)
		part, err := writer.CreatePart(header)
		if err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to create multipart image part"}
		}
		if _, err := part.Write(ref.Data); err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to write multipart image data"}
		}
	}
	if spec.Mask != nil {
		header := make(textproto.MIMEHeader)
		header.Set("Content-Disposition", `form-data; name="mask"; filename="mask.png"`)
		header.Set("Content-Type", "image/png")
		part, err := writer.CreatePart(header)
		if err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to create multipart mask part"}
		}
		if _, err := part.Write(spec.Mask.Data); err != nil {
			return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to write multipart mask data"}
		}
	}
	if err := writer.Close(); err != nil {
		return nil, "", &ImageError{Kind: ErrBadRequest, Message: "failed to finalize multipart request"}
	}
	return buf.Bytes(), writer.FormDataContentType(), nil
}

func sanitizeMultipartFilename(raw, fallback string) string {
	base := filepath.Base(strings.TrimSpace(raw))
	if base == "" || base == "." || base == "/" || base == `\` {
		base = fallback
	}
	var b strings.Builder
	for _, r := range base {
		if r == '"' || r == '\\' || r < 0x20 || r == 0x7f {
			b.WriteByte('_')
			continue
		}
		b.WriteRune(r)
	}
	if b.Len() == 0 {
		return fallback
	}
	return b.String()
}

func (c *Client) postMultipart(ctx context.Context, endpoint, apiKey, contentType string, payload []byte, timeout time.Duration) ([]byte, int, *ImageError) {
	reqCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(reqCtx, http.MethodPost, endpoint, bytes.NewReader(payload))
	if err != nil {
		return nil, 0, &ImageError{Kind: ErrBadRequest, Message: "invalid request URL"}
	}
	req.Header.Set("Content-Type", contentType)
	req.Header.Set("Accept", "application/json, text/event-stream")
	if key := strings.TrimSpace(apiKey); key != "" {
		req.Header.Set("Authorization", "Bearer "+key)
	}
	return c.execute(ctx, reqCtx, req, apiKey)
}

func (c *Client) postJSON(ctx context.Context, endpoint, apiKey string, payload map[string]any, timeout time.Duration) ([]byte, int, *ImageError) {
	encoded, err := json.Marshal(payload)
	if err != nil {
		return nil, 0, &ImageError{Kind: ErrBadRequest, Message: "failed to encode request"}
	}
	reqCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(reqCtx, http.MethodPost, endpoint, bytes.NewReader(encoded))
	if err != nil {
		return nil, 0, &ImageError{Kind: ErrBadRequest, Message: "invalid request URL"}
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	if key := strings.TrimSpace(apiKey); key != "" {
		req.Header.Set("Authorization", "Bearer "+key)
	}
	return c.execute(ctx, reqCtx, req, apiKey)
}

func (c *Client) getJSON(ctx context.Context, endpoint, apiKey string, timeout time.Duration) ([]byte, int, *ImageError) {
	reqCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(reqCtx, http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, 0, &ImageError{Kind: ErrBadRequest, Message: "invalid request URL"}
	}
	req.Header.Set("Accept", "application/json")
	if key := strings.TrimSpace(apiKey); key != "" {
		req.Header.Set("Authorization", "Bearer "+key)
	}
	return c.execute(ctx, reqCtx, req, apiKey)
}

func (c *Client) execute(parentCtx, reqCtx context.Context, req *http.Request, secret string) ([]byte, int, *ImageError) {
	resp, err := c.httpClient.Do(req)
	if err != nil {
		if errors.Is(parentCtx.Err(), context.Canceled) {
			return nil, 0, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
		}
		if errors.Is(reqCtx.Err(), context.DeadlineExceeded) {
			return nil, 0, &ImageError{Kind: ErrTimeout, Message: "request timed out"}
		}
		return nil, 0, &ImageError{Kind: ErrNetwork, Message: sanitizeSecret(err.Error(), secret)}
	}
	defer resp.Body.Close()
	limit := int64(maxSyncBodyBytes)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		limit = MaxErrorBodyBytes
	}
	raw, readErr := io.ReadAll(io.LimitReader(resp.Body, limit+1))
	if readErr != nil {
		if errors.Is(parentCtx.Err(), context.Canceled) {
			return nil, 0, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
		}
		if errors.Is(reqCtx.Err(), context.DeadlineExceeded) {
			return nil, 0, &ImageError{Kind: ErrTimeout, Message: "request timed out"}
		}
		return nil, resp.StatusCode, &ImageError{Kind: ErrNetwork, Status: resp.StatusCode, Message: sanitizeSecret(readErr.Error(), secret)}
	}
	return raw, resp.StatusCode, nil
}

// SaveOutputs writes every ImageOutput to <outputDir>/<stem>-<idx>.<ext>.
// URL outputs are downloaded without sending any API key.
func (c *Client) SaveOutputs(ctx context.Context, outputs []ImageOutput, outputDir, stem string) ([]string, *ImageError) {
	if err := os.MkdirAll(outputDir, 0o755); err != nil {
		return nil, &ImageError{Kind: ErrOther, Message: "failed to create output directory"}
	}
	paths := make([]string, 0, len(outputs))
	for i, output := range outputs {
		if ctx.Err() != nil {
			return nil, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
		}
		base := filepath.Join(outputDir, fmt.Sprintf("%s-%d", stem, i+1))
		saved, err := c.saveOne(ctx, output, base)
		if err != nil {
			return nil, err
		}
		paths = append(paths, saved)
	}
	return paths, nil
}

// DecodeOutput resolves one ImageOutput into raw bytes, downloading URL outputs
// without sending any API key and enforcing MaxDownloadBytes.
func (c *Client) DecodeOutput(ctx context.Context, output ImageOutput) (DecodedImage, *ImageError) {
	var data []byte
	if output.Source.Base64 != "" {
		cleaned := strings.Map(func(r rune) rune {
			if unicode.IsSpace(r) {
				return -1
			}
			return r
		}, output.Source.Base64)
		decoded, err := base64.StdEncoding.DecodeString(cleaned)
		if err != nil {
			decoded, err = base64.RawStdEncoding.DecodeString(cleaned)
			if err != nil {
				return DecodedImage{}, &ImageError{Kind: ErrOther, Message: "invalid base64 image payload"}
			}
		}
		data = decoded
	} else if output.Source.URL != "" {
		parsed, err := url.Parse(output.Source.URL)
		if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" {
			return DecodedImage{}, &ImageError{Kind: ErrOther, Message: "invalid image download URL"}
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, output.Source.URL, nil)
		if err != nil {
			return DecodedImage{}, &ImageError{Kind: ErrOther, Message: "invalid image download request"}
		}
		resp, err := c.downloadClient.Do(req)
		if err != nil {
			if errors.Is(ctx.Err(), context.Canceled) {
				return DecodedImage{}, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
			}
			return DecodedImage{}, &ImageError{Kind: ErrNetwork, Message: "failed to download generated image"}
		}
		defer resp.Body.Close()
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			return DecodedImage{}, &ImageError{Kind: ErrNetwork, Status: resp.StatusCode, Message: fmt.Sprintf("image download returned HTTP %d", resp.StatusCode)}
		}
		downloaded, err := io.ReadAll(io.LimitReader(resp.Body, MaxDownloadBytes+1))
		if err != nil {
			if errors.Is(ctx.Err(), context.Canceled) {
				return DecodedImage{}, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
			}
			return DecodedImage{}, &ImageError{Kind: ErrNetwork, Message: "failed while reading downloaded image"}
		}
		if len(downloaded) > MaxDownloadBytes {
			return DecodedImage{}, &ImageError{Kind: ErrOther, Message: "downloaded image exceeds size limit"}
		}
		data = downloaded
	} else {
		return DecodedImage{}, &ImageError{Kind: ErrOther, Message: "empty image output"}
	}
	ext := SniffExtension(data)
	return DecodedImage{
		Data:          data,
		Ext:           ext,
		MimeType:      MimeTypeForExt(ext),
		RevisedPrompt: output.RevisedPrompt,
	}, nil
}

func (c *Client) saveOne(ctx context.Context, output ImageOutput, stemPath string) (string, *ImageError) {
	decoded, err := c.DecodeOutput(ctx, output)
	if err != nil {
		return "", err
	}
	target := stemPath + "." + decoded.Ext
	tmp := stemPath + ".part"
	if err := os.WriteFile(tmp, decoded.Data, 0o644); err != nil {
		return "", &ImageError{Kind: ErrOther, Message: "failed to write image file"}
	}
	if err := os.Rename(tmp, target); err != nil {
		_ = os.Remove(tmp)
		return "", &ImageError{Kind: ErrOther, Message: "failed to finalize image file"}
	}
	return target, nil
}

// ParseOutputs reads either a JSON OpenAI Images response or an SSE stream.
func ParseOutputs(body []byte, secret string) ([]ImageOutput, *ImageError) {
	trimmed := bytes.TrimSpace(body)
	if len(trimmed) == 0 {
		return nil, &ImageError{Kind: ErrOther, Message: "empty response from image endpoint"}
	}
	if trimmed[0] == '{' {
		var obj map[string]any
		if err := json.Unmarshal(trimmed, &obj); err != nil {
			return nil, &ImageError{Kind: ErrOther, Message: "response was not valid JSON"}
		}
		return outputsFromMap(obj, secret)
	}
	return outputsFromSSE(trimmed, secret)
}

func outputsFromMap(obj map[string]any, secret string) ([]ImageOutput, *ImageError) {
	if rawErr, exists := obj["error"]; exists && rawErr != nil {
		switch v := rawErr.(type) {
		case string:
			return nil, classifyFields(0, "", "", v, secret)
		case map[string]any:
			return nil, classifyObject(0, v, secret)
		}
	}
	rawData, _ := obj["data"].([]any)
	var outputs []ImageOutput
	for _, entry := range rawData {
		m, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		if out, ok := outputFromMap(m); ok {
			outputs = append(outputs, out)
		}
	}
	if len(outputs) == 0 {
		return nil, &ImageError{Kind: ErrOther, Message: "endpoint returned no images"}
	}
	return outputs, nil
}

func outputFromMap(m map[string]any) (ImageOutput, bool) {
	strField := func(key string) string {
		s, _ := m[key].(string)
		return strings.TrimSpace(s)
	}
	var src ImageSource
	if b64 := strField("b64_json"); b64 != "" {
		src.Base64 = b64
	} else if u := strField("url"); u != "" {
		if strings.HasPrefix(u, "data:") {
			if _, payload, ok := strings.Cut(u, ","); ok && strings.TrimSpace(payload) != "" {
				src.Base64 = strings.TrimSpace(payload)
			} else {
				return ImageOutput{}, false
			}
		} else {
			src.URL = u
		}
	} else {
		return ImageOutput{}, false
	}
	return ImageOutput{
		Source:        src,
		RevisedPrompt: strField("revised_prompt"),
	}, true
}

func outputsFromSSE(body []byte, secret string) ([]ImageOutput, *ImageError) {
	var outputs []ImageOutput
	var eventName string
	var dataBuf strings.Builder

	flush := func() *ImageError {
		if dataBuf.Len() == 0 {
			return nil
		}
		payload := dataBuf.String()
		dataBuf.Reset()
		var obj map[string]any
		if err := json.Unmarshal([]byte(payload), &obj); err != nil {
			return nil
		}
		kind, _ := obj["type"].(string)
		if eventName == "error" || kind == "error" || obj["error"] != nil {
			errObj := obj["error"]
			if errObj == nil {
				errObj = obj
			}
			switch v := errObj.(type) {
			case string:
				return classifyFields(0, "", "", v, secret)
			case map[string]any:
				return classifyObject(0, v, secret)
			}
		}
		if strings.HasSuffix(eventName, ".completed") || strings.HasSuffix(kind, ".completed") {
			if out, ok := outputFromMap(obj); ok {
				outputs = append(outputs, out)
			}
		}
		return nil
	}

	scanner := bufio.NewScanner(bytes.NewReader(body))
	scanner.Buffer(make([]byte, 64*1024), maxSyncBodyBytes)
	for scanner.Scan() {
		line := strings.TrimRight(scanner.Text(), "\r")
		if line == "" {
			if err := flush(); err != nil {
				return nil, err
			}
			eventName = ""
			continue
		}
		if after, ok := strings.CutPrefix(line, "event:"); ok {
			eventName = strings.TrimSpace(after)
		} else if after, ok := strings.CutPrefix(line, "data:"); ok {
			if dataBuf.Len() > 0 {
				dataBuf.WriteByte('\n')
			}
			dataBuf.WriteString(strings.TrimLeft(after, " \t"))
		}
	}
	if err := flush(); err != nil {
		return nil, err
	}
	if len(outputs) == 0 {
		return nil, &ImageError{Kind: ErrOther, Message: "stream ended without an image"}
	}
	return outputs, nil
}

func parseTaskResponse(body []byte, secret string) (bool, []ImageOutput, *ImageError) {
	var obj map[string]any
	if err := json.Unmarshal(body, &obj); err != nil {
		return false, nil, &ImageError{Kind: ErrOther, Message: "task record was not JSON"}
	}
	status, _ := obj["status"].(string)
	httpStatus := int(floatField(obj, "http_status"))
	switch status {
	case "processing", "queued", "running":
		return false, nil, nil
	case "failed":
		var taskErr *ImageError
		switch v := obj["error"].(type) {
		case map[string]any:
			taskErr = classifyObject(httpStatus, v, secret)
		case string:
			taskErr = classifyFields(httpStatus, "", "", v, secret)
		default:
			taskErr = classifyFields(httpStatus, "", "", "image task failed", secret)
		}
		taskErr.TaskEnded = true
		return false, nil, taskErr
	case "completed":
		if resultMap, ok := obj["result"].(map[string]any); ok {
			outputs, err := outputsFromMap(resultMap, secret)
			if err == nil {
				return true, outputs, nil
			}
			if err.Kind != ErrOther {
				return false, nil, err
			}
		}
		if imgURL, ok := obj["image_url"].(string); ok && strings.TrimSpace(imgURL) != "" {
			return true, []ImageOutput{{Source: ImageSource{URL: strings.TrimSpace(imgURL)}}}, nil
		}
		return false, nil, &ImageError{Kind: ErrOther, Message: "task completed without an image"}
	default:
		return false, nil, &ImageError{Kind: ErrOther, Message: "unknown image task status"}
	}
}

// ClassifyError inspects an HTTP error body (OpenAI or CheapRouter shape) and
// returns a sanitized, categorized ImageError.
func ClassifyError(status int, body []byte, secret string) *ImageError {
	var obj map[string]any
	if json.Unmarshal(body, &obj) == nil {
		if rawErr, ok := obj["error"]; ok && rawErr != nil {
			switch v := rawErr.(type) {
			case map[string]any:
				return classifyObject(status, v, secret)
			case string:
				return classifyFields(status, "", "", v, secret)
			}
		}
		if msg, ok := obj["message"].(string); ok && strings.TrimSpace(msg) != "" {
			return classifyFields(status, "", "", msg, secret)
		}
	}
	text := strings.TrimSpace(string(body))
	if text == "" {
		text = fmt.Sprintf("HTTP %d", status)
	}
	return classifyFields(status, "", "", text, secret)
}

func classifyObject(status int, obj map[string]any, secret string) *ImageError {
	strVal := func(key string) string {
		switch v := obj[key].(type) {
		case string:
			return v
		case float64:
			return fmt.Sprintf("%.0f", v)
		default:
			return ""
		}
	}
	if status == 0 {
		status = int(floatField(obj, "http_status"))
	}
	msg := strings.TrimSpace(strVal("message"))
	if msg == "" {
		msg = "the server gave no reason"
	}
	return classifyFields(status, strVal("type"), strVal("code"), msg, secret)
}

func classifyFields(status int, errType, errCode, message, secret string) *ImageError {
	clean := truncateRunes(sanitizeSecret(strings.TrimSpace(message), secret), 300)
	lower := strings.ToLower(clean)
	is := func(name string) bool {
		return strings.EqualFold(errType, name) || strings.EqualFold(errCode, name)
	}

	var kind ErrorKind
	switch {
	case strings.Contains(lower, "async image tasks are not enabled"):
		kind = ErrAsyncUnavailable
	case strings.Contains(lower, "image generation is not enabled"):
		kind = ErrNoImagePermission
	case is("content_policy_violation") || is("moderation_blocked") ||
		strings.Contains(lower, "content policy") || strings.Contains(lower, "safety system"):
		kind = ErrContentPolicy
	case strings.Contains(lower, "not supported for this platform"):
		kind = ErrWrongPlatform
	case status == http.StatusPaymentRequired:
		kind = ErrUpstreamBalance
	case is("billing_error") || is("insufficient_quota") ||
		strings.Contains(lower, "insufficient balance") || strings.Contains(lower, "insufficient quota") ||
		strings.Contains(lower, "exceeded your current quota") || strings.Contains(lower, "billing hard limit"):
		kind = ErrOwnBalance
	case is("image_task_not_found"):
		kind = ErrTaskLost
	case status == http.StatusUnauthorized || status == http.StatusForbidden ||
		is("authentication_error") || is("invalid_api_key") ||
		strings.Contains(lower, "invalid api key") || strings.Contains(lower, "incorrect api key"):
		kind = ErrUnauthorized
	case status == http.StatusNotFound || is("model_not_found"):
		kind = ErrModelUnavailable
	case status == http.StatusTooManyRequests || is("rate_limit_error") || is("rate_limit_exceeded") ||
		is("gateway_concurrency_limit") || is("gateway_queue_full"):
		kind = ErrBusy
	case status == http.StatusGatewayTimeout || status == 524 || is("timeout_error"):
		kind = ErrTimeout
	case status == http.StatusBadRequest || status == http.StatusUnprocessableEntity ||
		is("invalid_request_error") || is("invalid_value") || strings.Contains(lower, "unsupported"):
		kind = ErrBadRequest
	default:
		kind = ErrOther
	}
	return &ImageError{Kind: kind, Status: status, Message: clean}
}

func sanitizeSecret(text, secret string) string {
	s := strings.TrimSpace(secret)
	if s == "" || len(s) < 4 {
		return text
	}
	return strings.ReplaceAll(text, s, "[redacted]")
}

func truncateRunes(s string, maxRunes int) string {
	runes := []rune(s)
	if len(runes) <= maxRunes {
		return s
	}
	return string(runes[:maxRunes]) + "…"
}

func floatField(m map[string]any, key string) float64 {
	v, _ := m[key].(float64)
	return v
}
