package cheapauth

import (
	"slices"
	"strconv"
	"strings"
)

// ImageGroup is one CheapRouter routing group candidate for image generation.
type ImageGroup struct {
	GroupID        int64
	Name           string
	Platform       string
	RateMultiplier float64
}

// IsGrokImageModel reports whether model belongs to the Grok image family.
func IsGrokImageModel(model string) bool {
	name := normalizedBareModelID(model)
	return strings.HasPrefix(name, "grok-")
}

// normalizedBareModelID strips optional lane ("lane::") and provider ("openai/") prefixes.
func normalizedBareModelID(model string) string {
	name := strings.ToLower(strings.TrimSpace(model))
	if _, bare, ok := strings.Cut(name, "::"); ok {
		name = strings.TrimSpace(bare)
	}
	name = strings.TrimPrefix(name, "models/")
	if idx := strings.LastIndex(name, "/"); idx >= 0 {
		name = name[idx+1:]
	}
	return strings.TrimSpace(name)
}

func isNonImageMediaName(name string) bool {
	if name == "" {
		return true
	}
	for _, prefix := range []string{
		"grok-imagine-video",
		"sora",
		"veo-",
		"kling-",
		"runway-",
		"tts-",
		"gpt-4o-mini-tts",
		"whisper",
	} {
		if strings.HasPrefix(name, prefix) {
			return true
		}
	}
	for _, word := range strings.FieldsFunc(name, func(r rune) bool {
		return r == '-' || r == '.' || r == '/' || r == ':' || r == '_'
	}) {
		switch word {
		case "video", "embedding", "embeddings", "moderation", "rerank", "tts", "whisper", "audio":
			return true
		}
	}
	return false
}

// IsImageModelName reports whether model matches the CheapRouter/OpenAI image
// model naming rules (aligned with Rust sub2api::images::is_image_model plus
// common OpenAI-compatible image model families).
func IsImageModelName(model string) bool {
	name := normalizedBareModelID(model)
	if isNonImageMediaName(name) {
		return false
	}
	if strings.HasPrefix(name, "gpt-image-") || name == "gpt-image" {
		return true
	}
	if name == "grok-imagine" || name == "grok-imagine-edit" || strings.HasPrefix(name, "grok-imagine-image") {
		return true
	}
	if strings.HasPrefix(name, "grok-image") || strings.HasPrefix(name, "grok-2-image") {
		return true
	}
	if strings.HasPrefix(name, "dall-e-") || name == "dall-e" {
		return true
	}
	if strings.HasPrefix(name, "imagen-") || strings.HasPrefix(name, "flux-") || strings.HasPrefix(name, "flux.") {
		return true
	}
	if strings.HasPrefix(name, "sdxl") || strings.HasPrefix(name, "stable-diffusion") {
		return true
	}
	return false
}

func isSupportedImagePlatform(platform string) bool {
	switch strings.ToLower(strings.TrimSpace(platform)) {
	case "", "openai", "grok", "xai", "x-ai", "composite", "image", "images", "google", "gemini", "vertex":
		return true
	default:
		return false
	}
}

// IsImageCatalogModel reports whether a CheapRouter catalog entry is an image
// generation model, combining billing_mode, pricing_details, platform, and model
// naming rules. Video, audio, embedding, and chat-only entries are excluded.
func IsImageCatalogModel(m Model) bool {
	id := strings.TrimSpace(m.ID)
	if id == "" {
		return false
	}
	name := normalizedBareModelID(id)
	if isNonImageMediaName(name) {
		return false
	}
	unit := strings.ToLower(strings.TrimSpace(m.PricingDetails.MediaUnit))
	if unit == "second" || unit == "video" || unit == "minute" {
		return false
	}
	mode := strings.ToLower(strings.TrimSpace(m.BillingMode))
	if mode == "image" || unit == "image" || m.EffectivePricing.PerImageUSD != nil {
		return true
	}
	if !IsImageModelName(id) {
		return false
	}
	return isSupportedImagePlatform(m.Platform)
}

// ImageCatalogModels extracts and deduplicates image generation models from a
// CheapRouter model catalog, sorted deterministically by model ID.
func ImageCatalogModels(catalog []Model) []Model {
	if len(catalog) == 0 {
		return nil
	}
	byKey := make(map[string]Model)
	for _, item := range catalog {
		if !IsImageCatalogModel(item) {
			continue
		}
		item.ID = strings.TrimSpace(item.ID)
		item.DisplayName = strings.TrimSpace(item.DisplayName)
		key := strings.ToLower(item.ID)
		existing, ok := byKey[key]
		if !ok {
			byKey[key] = item
			continue
		}
		if existing.DisplayName == "" && item.DisplayName != "" {
			existing.DisplayName = item.DisplayName
		}
		if existing.Platform == "" || strings.EqualFold(existing.Platform, "composite") {
			if item.Platform != "" && !strings.EqualFold(item.Platform, "composite") {
				existing.Platform = item.Platform
			}
		}
		if existing.BillingMode == "" && item.BillingMode != "" {
			existing.BillingMode = item.BillingMode
		}
		if len(existing.PricingDetails.MediaTiers) == 0 && len(item.PricingDetails.MediaTiers) > 0 {
			existing.PricingDetails = item.PricingDetails
		}
		if existing.EffectivePricing.PerImageUSD == nil && item.EffectivePricing.PerImageUSD != nil {
			existing.EffectivePricing.PerImageUSD = item.EffectivePricing.PerImageUSD
		}
		byKey[key] = existing
	}
	if len(byKey) == 0 {
		return nil
	}
	out := make([]Model, 0, len(byKey))
	for _, item := range byKey {
		out = append(out, item)
	}
	slices.SortFunc(out, func(a, b Model) int {
		return strings.Compare(strings.ToLower(a.ID), strings.ToLower(b.ID))
	})
	return out
}

// ImageRouteCandidates mirrors Rust sub2api::images::image_route_candidates:
// remembered group for the model, active lane binding for its platform,
// catalog groups (own platform before composite, then cheaper rate multiplier),
// and finally legacy GroupID.
func ImageRouteCandidates(credentials Credentials, catalog []Model, model string) []ImageGroup {
	wanted := strings.ToLower(strings.TrimSpace(model))
	if wanted == "" {
		return nil
	}
	ownPlatform := "openai"
	ownLane := LaneCodex
	if IsGrokImageModel(wanted) {
		ownPlatform = "grok"
		ownLane = LaneGrok
	}

	var listed []ImageGroup
	addListed := func(groupID int64, name, platform string, rate float64) {
		if groupID <= 0 {
			return
		}
		plat := strings.ToLower(strings.TrimSpace(platform))
		switch plat {
		case "", "openai", "grok", "xai", "x-ai", "composite":
			if plat == "xai" || plat == "x-ai" {
				plat = "grok"
			}
		default:
			return
		}
		for i := range listed {
			if listed[i].GroupID == groupID {
				if listed[i].Name == "" && name != "" {
					listed[i].Name = name
				}
				if listed[i].Platform == "" && plat != "" {
					listed[i].Platform = plat
				}
				return
			}
		}
		listed = append(listed, ImageGroup{
			GroupID:        groupID,
			Name:           strings.TrimSpace(name),
			Platform:       plat,
			RateMultiplier: rate,
		})
	}

	for _, item := range catalog {
		if !strings.EqualFold(strings.TrimSpace(item.ID), wanted) {
			continue
		}
		if item.BestGroup.ID > 0 {
			addListed(item.BestGroup.ID, item.BestGroup.Name, item.Platform, item.BestGroup.RateMultiplier)
		}
		for _, other := range item.OtherGroups {
			if other.Group.ID > 0 {
				addListed(other.Group.ID, other.Group.Name, item.Platform, other.Group.RateMultiplier)
			}
		}
	}

	rank := func(g ImageGroup) int {
		if g.Platform == ownPlatform {
			return 0
		}
		if g.Platform == "" {
			return 1
		}
		return 2
	}
	slices.SortFunc(listed, func(a, b ImageGroup) int {
		if ra, rb := rank(a), rank(b); ra != rb {
			return ra - rb
		}
		if a.RateMultiplier != b.RateMultiplier {
			if a.RateMultiplier < b.RateMultiplier {
				return -1
			}
			return 1
		}
		if a.GroupID < b.GroupID {
			return -1
		}
		if a.GroupID > b.GroupID {
			return 1
		}
		return 0
	})

	known := func(id int64) ImageGroup {
		for _, g := range listed {
			if g.GroupID == id {
				return g
			}
		}
		return ImageGroup{GroupID: id}
	}

	var order []ImageGroup
	seen := make(map[int64]bool)
	push := func(id int64) {
		if id <= 0 || seen[id] {
			return
		}
		seen[id] = true
		order = append(order, known(id))
	}

	for name, gid := range credentials.ImageGroups {
		if strings.EqualFold(strings.TrimSpace(name), wanted) {
			push(gid)
			break
		}
	}
	if binding, ok := credentials.LaneBindings[ownLane]; ok && binding.GroupID > 0 {
		push(binding.GroupID)
	}
	for _, g := range listed {
		push(g.GroupID)
	}
	if credentials.GroupID != nil && *credentials.GroupID > 0 {
		push(*credentials.GroupID)
	}
	return order
}

// TierForSize mirrors Rust sub2api::images::tier_for_size ("1K", "2K", "4K").
func TierForSize(size string) string {
	s := strings.ToLower(strings.TrimSpace(size))
	switch s {
	case "1k":
		return "1K"
	case "2k", "2048x2048", "2048x1152":
		return "2K"
	case "4k", "3840x2160", "2160x3840":
		return "4K"
	}
	if wStr, hStr, ok := strings.Cut(s, "x"); ok {
		w, errW := strconv.Atoi(strings.TrimSpace(wStr))
		h, errH := strconv.Atoi(strings.TrimSpace(hStr))
		if errW == nil && errH == nil {
			edge := max(w, h)
			switch {
			case edge <= 0:
				return "2K"
			case edge <= 1024:
				return "1K"
			case edge <= 2048:
				return "2K"
			default:
				return "4K"
			}
		}
	}
	return "2K"
}

// EstimateImageUSD computes an estimated USD cost for count images at size from
// catalog pricing metadata, returning (0, false) when pricing is unknown.
func EstimateImageUSD(item Model, size string, count int) (float64, bool) {
	if count < 1 {
		count = 1
	}
	tier := TierForSize(size)
	for _, entry := range item.PricingDetails.MediaTiers {
		if strings.EqualFold(strings.TrimSpace(entry.Tier), tier) && entry.EffectiveUSD != nil {
			return *entry.EffectiveUSD * float64(count), true
		}
	}
	if len(item.PricingDetails.MediaTiers) == 0 && item.EffectivePricing.PerImageUSD != nil {
		return *item.EffectivePricing.PerImageUSD * float64(count), true
	}
	return 0, false
}
