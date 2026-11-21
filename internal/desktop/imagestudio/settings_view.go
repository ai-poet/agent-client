package imagestudio

import (
	"strings"

	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/icons"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

// SettingsPage renders Settings -> Image API for configuring a custom
// OpenAI-compatible image generation endpoint alongside CheapRouter.
func SettingsPage(c *ui.Context, state *SettingsState) {
	t := func(k string) string { return i18n.T(c, k) }
	if state == nil {
		kit.Result(c, kit.ResultProps{
			Icon:     icons.FileImage,
			Title:    t("settings_image_api"),
			SubTitle: t("unavailable"),
			Card:     true,
		})
		return
	}

	kit.Page(c, "settings-image-api", t("settings_image_api"), t("settings_image_api_description"), func() {
		statusTone := kit.Neutral
		statusLabel := t("settings_image_api_status_empty")
		if state.Configured {
			statusTone = kit.Positive
			statusLabel = t("settings_image_api_status_ready")
		}

		kit.Group(c, "custom-api-form", "", "", func() {
			kit.Badge(c, statusLabel, statusTone)
			if state.OpenStudio != nil {
				kit.Secondary(c.Key("open-studio"), t("settings_image_api_open_studio")).OnClick(state.OpenStudio)
			}
		}, func() {
			kit.Card(c, func() {
				ui.Column(c.Key("fields")).Padding(16, 0).Gap(16).Children(func() {
					if state.ErrorKey != "" {
						kit.Alert(c, "save-error", kit.Negative, t(state.ErrorKey), nil)
					}

					kit.Field(c, "base-url", t("settings_image_api_base_url"), t("settings_image_api_base_url_desc"), func() {
						ui.TextInput(c.Key("input"), &state.BaseURL).
							Label(t("settings_image_api_base_url")).
							Placeholder(t("settings_image_api_base_url_placeholder")).
							FillWidth().
							Disabled(state.Saving)
					})

					keyPlaceholder := t("settings_image_api_key_placeholder")
					if state.HasSavedKey {
						keyPlaceholder = t("settings_image_api_key_saved_placeholder")
					}
					kit.Field(c, "api-key", t("settings_image_api_key"), t("settings_image_api_key_desc"), func() {
						ui.Column(c).FillWidth().Gap(8).Children(func() {
							kit.SecretInput(c.Key("secret"), &state.APIKeyInput, &state.APIKeyShown, kit.SecretInputProps{
								Label:       t("settings_image_api_key"),
								Placeholder: keyPlaceholder,
								Disabled:    state.Saving,
							}).FillWidth()
							if state.HasSavedKey {
								ui.Row(c.Key("key-status")).Gap(8).AlignItems(ui.Center).Children(func() {
									kit.Badge(c, t("settings_image_api_key_saved_badge"), kit.Positive)
									if state.ClearKey != nil {
										kit.TextAction(c.Key("clear-key"), t("settings_image_api_clear_key"), state.Saving).OnClick(state.ClearKey)
									}
								})
							}
						})
					})

					kit.Field(c, "model", t("settings_image_api_model"), t("settings_image_api_model_desc"), func() {
						ui.TextInput(c.Key("input"), &state.Model).
							Label(t("settings_image_api_model")).
							Placeholder(t("settings_image_api_model_placeholder")).
							FillWidth().
							Disabled(state.Saving)
					})

					kit.Field(c, "extra-params", t("settings_image_api_extra"), t("settings_image_api_extra_desc"), func() {
						ui.TextArea(c.Key("input"), &state.ExtraParams).
							Label(t("settings_image_api_extra")).
							Placeholder(t("settings_image_api_extra_placeholder")).
							FillWidth().
							MinHeight(84).
							Font("monospace").
							Disabled(state.Saving)
					})

					canSave := strings.TrimSpace(state.BaseURL) != "" && strings.TrimSpace(state.Model) != ""
					ui.Row(c.Key("actions")).Gap(8).AlignItems(ui.Center).Children(func() {
						kit.PrimaryBusy(c.Key("save"), t("settings_image_api_save"), !canSave, state.Saving).OnClick(func() {
							if state.Save != nil {
								state.Save()
							}
						})
						if (state.Configured || state.HasSavedKey || strings.TrimSpace(state.BaseURL) != "" || strings.TrimSpace(state.Model) != "") && state.ClearAll != nil {
							kit.Secondary(c.Key("clear"), t("settings_image_api_clear")).Disabled(state.Saving).OnClick(state.ClearAll)
						}
					})
				})
			})
		})
	})
}
