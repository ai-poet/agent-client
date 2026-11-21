package migrate

import (
	"encoding/json"
	"errors"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type legacyAccountMetadata struct {
	Version         int    `json:"version"`
	AccountID       string `json:"account_id,omitempty"`
	Endpoint        string `json:"endpoint"`
	ReloginRequired bool   `json:"relogin_required"`
}

func migrateCredentials(profile, source string) (CredentialReport, []Warning, error) {
	report := CredentialReport{Found: true, ReloginRequired: true, SecretsMigrated: false}
	raw, err := os.ReadFile(source)
	if err != nil {
		return report, nil, fail("credentials", "read account metadata", err)
	}
	// Decode only allowlisted non-secret fields. In particular, do not unmarshal
	// into the legacy Credentials type or copy the source JSON to the target.
	var fields struct {
		AccountID json.RawMessage `json:"account_id"`
		UserID    json.RawMessage `json:"user_id"`
		ID        json.RawMessage `json:"id"`
		SessionID json.RawMessage `json:"session_id"`
		Email     string          `json:"email"`
		Endpoint  string          `json:"endpoint"`
	}
	if err := json.Unmarshal(raw, &fields); err != nil {
		report.Found = true
		return report, []Warning{{Stage: "credentials", Message: "credential file could not be parsed; account metadata was not migrated"}}, nil
	}
	for _, candidate := range []json.RawMessage{fields.AccountID, fields.UserID, fields.ID, fields.SessionID} {
		if len(candidate) == 0 {
			continue
		}
		report.AccountID = jsonIdentifier(candidate)
		if report.AccountID != "" {
			break
		}
	}
	if report.AccountID == "" {
		report.AccountID = strings.TrimSpace(fields.Email)
	}
	report.Endpoint = strings.TrimRight(strings.TrimSpace(fields.Endpoint), "/")
	if report.Endpoint != "" {
		parsed, err := url.Parse(report.Endpoint)
		if err != nil || parsed.Host == "" || parsed.Scheme != "https" && parsed.Scheme != "http" {
			report.Endpoint = ""
		}
	}
	if report.Endpoint == "" {
		return report, []Warning{{Stage: "credentials", Message: "legacy account has no usable endpoint; sign in again"}}, nil
	}
	warnings := []Warning{}
	if report.AccountID == "" {
		warnings = append(warnings, Warning{Stage: "credentials", Message: "legacy account has no usable identifier; only its endpoint was retained"})

	}

	destination := filepath.Join(profile, AccountFilename)
	report.Destination = destination
	if _, err := os.Stat(destination); err == nil {
		report.Migrated = true
		return report, warnings, nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return report, nil, fail("credentials", "inspect account metadata destination", err)
	}
	metadata := legacyAccountMetadata{Version: Version, AccountID: report.AccountID, Endpoint: report.Endpoint, ReloginRequired: true}
	if err := writeJSONAtomic(destination, metadata); err != nil {
		return report, nil, fail("credentials", "write account metadata", err)
	}
	report.Migrated = true
	return report, warnings, nil
}

func jsonIdentifier(raw json.RawMessage) string {
	var text string
	if json.Unmarshal(raw, &text) == nil {
		return strings.TrimSpace(text)
	}
	var number json.Number
	if json.Unmarshal(raw, &number) == nil {
		return number.String()
	}
	return ""
}

// NormalizeLegacyLanguage maps legacy CheapRouter / Waku language values
// (including Rust AppLanguage kebab-case values like "simplified-chinese")
// into Sailry's validated preference value and corresponding BCP 47 locale.
// Invalid or unsupported values return ok == false so no illegal value is stored.
func NormalizeLegacyLanguage(raw string) (preference string, locale string, ok bool) {
	key := strings.ToLower(strings.TrimSpace(raw))
	key = strings.ReplaceAll(key, "_", "-")
	switch key {
	case "system":
		return "system", "", true
	case "english", "en", "en-us", "en-gb":
		return "english", "en", true
	case "simplified-chinese", "chinese", "zh-cn", "zh-hans", "zh-sg":
		return "chinese", "zh-CN", true
	case "traditional-chinese", "zh-tw", "zh-hant", "zh-hk", "zh-mo":
		return "traditional_chinese", "zh-TW", true
	case "japanese", "ja", "ja-jp":
		return "japanese", "ja", true
	case "korean", "ko", "ko-kr":
		return "korean", "ko", true
	case "french", "fr", "fr-fr":
		return "french", "fr", true
	case "german", "de", "de-de":
		return "german", "de", true
	case "spanish", "es", "es-es":
		return "spanish", "es", true
	case "portuguese-brazil", "pt-br":
		return "portuguese_brazil", "pt-BR", true
	case "russian", "ru", "ru-ru":
		return "russian", "ru", true
	default:
		return "", "", false
	}
}

func migrateLegacySettings(backupRoot string, hasAppSettings, hasSettings bool) (map[string]any, []UnmappedField, []Warning) {
	merged := map[string]any{}
	unmapped := []UnmappedField{}
	warnings := []Warning{}
	if hasSettings {
		values, fileUnmapped, fileWarnings := migrateSettings(filepath.Join(backupRoot, legacySettingsName))
		for k, v := range values {
			merged[k] = v
		}
		unmapped = append(unmapped, fileUnmapped...)
		warnings = append(warnings, fileWarnings...)
	}
	if hasAppSettings {
		// app.json is the authoritative desktop preference file in main; its
		// valid keys override any older settings.json fallback.
		values, fileUnmapped, fileWarnings := migrateSettings(filepath.Join(backupRoot, legacyAppSettingsName))
		for k, v := range values {
			merged[k] = v
		}
		unmapped = append(unmapped, fileUnmapped...)
		warnings = append(warnings, fileWarnings...)
	}
	return merged, unmapped, warnings
}

func migrateSettings(source string) (map[string]any, []UnmappedField, []Warning) {
	settings := map[string]any{}
	unmapped := []UnmappedField{}
	raw, err := os.ReadFile(source)
	if err != nil {
		return settings, unmapped, []Warning{{Stage: "settings", Message: "settings file could not be read; Sailry preferences were preserved"}}
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return settings, unmapped, []Warning{{Stage: "settings", Message: "settings file could not be parsed; Sailry preferences were preserved"}}
	}
	for _, name := range []string{"theme", "language", "analytics_enabled"} {
		value, found := fields[name]
		if !found {
			continue
		}
		valid := false
		switch name {
		case "theme":
			var theme string
			if json.Unmarshal(value, &theme) == nil {
				switch strings.ToLower(strings.TrimSpace(theme)) {
				case "light", "dark", "system":
					settings["theme"] = strings.ToLower(strings.TrimSpace(theme))
					valid = true
				}
			}
		case "language":
			var language string
			if json.Unmarshal(value, &language) == nil {
				if normalized, _, ok := NormalizeLegacyLanguage(language); ok {
					settings["language"] = normalized
					valid = true
				}
			}
		case "analytics_enabled":
			var enabled bool
			if json.Unmarshal(value, &enabled) == nil {
				settings["analytics_enabled"] = enabled
				valid = true
			}
		}
		if !valid {
			unmapped = append(unmapped, UnmappedField{Entity: "settings", Field: name, Reason: "invalid or unsupported preference value"})
		}
	}
	known := map[string]bool{"theme": true, "language": true, "analytics_enabled": true, "version": true}
	unknown := []string{}
	for name := range fields {
		if !known[name] {
			unknown = append(unknown, name)
		}
	}
	sort.Strings(unknown)
	for _, name := range unknown {
		unmapped = append(unmapped, UnmappedField{Entity: "settings", Field: name, Reason: "no Sailry preference mapping"})
	}
	return settings, unmapped, nil
}
