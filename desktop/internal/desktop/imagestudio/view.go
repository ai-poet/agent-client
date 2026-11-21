package imagestudio

import (
	"fmt"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	core "github.com/ai-poet/agent-client/desktop/internal/imagestudio"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/icons"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

const (
	studioMaxWidth float32 = 1040
	studioCardW    float32 = 224
)

// Page renders the Creative Studio workspace in the shell's main column.
func Page(c *ui.Context, state *State) {
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }
	if state == nil {
		kit.Result(c, kit.ResultProps{
			Icon:     icons.Image,
			Title:    t("image_studio"),
			SubTitle: t("unavailable"),
		}).Grow(1).Justify(ui.Center)
		return
	}
	if state.HasRunningJobs() {
		c.After(time.Second)
	}

	ui.Column(c.Key("image-studio")).Fill().MinHeight(0).Children(func() {
		renderToolbar(c, state)
		ui.Scroll(c.Key("gallery-scroll")).Grow(1).MinHeight(0).FillWidth().TrackScroll(&state.Scroll).Children(func() {
			ui.Column(c.Key("gallery-wrap")).FillWidth().MaxWidth(studioMaxWidth).Margin(0, ui.Auto).Padding(12, 24, 24, 24).Gap(16).Children(func() {
				renderChannelBanner(c, state)
				renderGallery(c, state)
			})
		})
		renderComposer(c, state)
	})

	kit.ImageViewer(c, &state.Viewer, func(entry ViewerEntry) kit.ViewerItem {
		src := resolveThumb(state, entry.Path)
		return kit.ViewerItem{
			Source: src,
			Name:   filepath.Base(entry.Path),
		}
	}, func(act kit.ImageAction, entry ViewerEntry) {
		switch act {
		case kit.ImageCopy:
			if state.CopyImage != nil {
				state.CopyImage(entry.Path)
			}
		case kit.ImageSave:
			if state.SaveImageAs != nil {
				state.SaveImageAs(entry.Path)
			}
		case kit.ImageOpen:
			if state.OpenImage != nil {
				state.OpenImage(entry.Path)
			}
		}
	})
}

func renderToolbar(c *ui.Context, state *State) {
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }
	count := state.PictureCount()
	subtitle := t("image_studio_subtitle")
	if count > 0 {
		subtitle = t("image_studio_picture_count", "count", count)
	}

	selected := 0
	if state.EffectiveChannel() == core.ChannelCustom {
		selected = 1
	}

	ui.Row(c.Key("toolbar")).FillWidth().Shrink(0).BorderWidth(0, 0, 1, 0).BorderColor(p.Border).Children(func() {
		ui.Row(c).FillWidth().MaxWidth(studioMaxWidth).Margin(0, ui.Auto).Padding(10, 24).Gap(12).AlignItems(ui.Center).Wrap().Children(func() {
			ui.Text(c, subtitle).FontSize(13).TextColor(p.MutedForeground).Grow(1).Shrink(1).MinWidth(140)

			seg := kit.Segmented(c, "channel-switch", &selected, []string{
				t("image_studio_channel_cheaprouter"),
				t("image_studio_channel_custom"),
			}, kit.SegmentedProps{Size: kit.SizeSmall}).Label(t("image_studio_channel"))
			if seg.Changed() {
				if selected == 1 {
					state.Channel = core.ChannelCustom
				} else {
					state.Channel = core.ChannelCheapRouter
				}
				if state.OnPrefsChanged != nil {
					state.OnPrefsChanged()
				}
			}

			if state.OpenOutputFolder != nil {
				kit.Secondary(c.Key("open-folder"), t("image_studio_open_folder")).OnClick(state.OpenOutputFolder)
			}
			if state.OpenSettings != nil {
				kit.Secondary(c.Key("open-settings"), t("image_studio_open_settings")).OnClick(state.OpenSettings)
			}
		})
	})
}

func renderChannelBanner(c *ui.Context, state *State) {
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }

	if state.EffectiveChannel() == core.ChannelCustom {
		if !state.Custom.Configured {
			kit.Alert(c, "custom-unconfigured", kit.Info, t("image_studio_custom_unconfigured_notice"), func() {
				if state.OpenSettings != nil {
					kit.Primary(c.Key("go-settings"), t("image_studio_open_settings"), false).OnClick(state.OpenSettings)
				}
			})
		}
		return
	}

	if !state.CheapSignedIn {
		kit.Alert(c, "cheap-signed-out", kit.Info, t("image_studio_signed_out_notice"), func() {
			ui.Row(c).Gap(8).Shrink(0).Children(func() {
				if state.OpenCheapAccount != nil {
					kit.Primary(c.Key("sign-in"), t("image_studio_sign_in"), false).OnClick(state.OpenCheapAccount)
				}
				if state.OpenSettings != nil {
					kit.Secondary(c.Key("go-settings"), t("image_studio_open_settings")).OnClick(state.OpenSettings)
				}
			})
		})
		return
	}

	if state.CheapLoading {
		p := theme.Of(c)
		ui.Row(c.Key("cheap-loading")).Gap(8).AlignItems(ui.Center).Padding(8, 12).Radius(p.Radius).Background(p.Muted).Children(func() {
			kit.Busy(c)
			ui.Text(c, t("image_studio_models_loading")).FontSize(13).TextColor(p.MutedForeground)
		})
		return
	}

	if len(state.CheapModels) == 0 {
		kit.Alert(c, "cheap-no-models", kit.Caution, t("image_studio_no_models_notice"), func() {
			ui.Row(c).Gap(8).Shrink(0).Children(func() {
				if state.OpenSettings != nil {
					kit.Primary(c.Key("go-settings"), t("image_studio_open_settings"), false).OnClick(state.OpenSettings)
				}
				if state.RefreshModels != nil {
					kit.Secondary(c.Key("refresh"), t("image_studio_refresh_models")).OnClick(state.RefreshModels)
				}
			})
		})
	}
}

func renderGallery(c *ui.Context, state *State) {
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }
	if len(state.Jobs) == 0 {
		kit.Result(c, kit.ResultProps{
			Icon:     icons.Image,
			Title:    t("image_studio_empty_title"),
			SubTitle: t("image_studio_empty_body"),
		}).Padding(48, 0)
		return
	}

	nowUnix := c.Now().Unix()
	allEntries := state.AllViewerEntries()
	entryIndex := make(map[string]int, len(allEntries))
	for i, entry := range allEntries {
		entryIndex[entry.JobID+"|"+entry.Path] = i
	}

	ui.Row(c.Key("cards")).FillWidth().Wrap().Gap(16).AlignItems(ui.Start).Children(func() {
		for i := range state.Jobs {
			job := &state.Jobs[i]
			switch job.Status {
			case core.JobDone:
				for outIdx, path := range job.Outputs {
					idx := entryIndex[job.ID+"|"+path]
					renderDoneCard(c, state, job, outIdx, path, allEntries, idx)
				}
			case core.JobRunning:
				renderRunningCard(c, state, job, nowUnix-job.StartedAt)
			case core.JobFailed, core.JobCancelled:
				renderFailedCard(c, state, job)
			}
		}
	})
}

func renderDoneCard(c *ui.Context, state *State, job *core.Job, outIdx int, path string, allEntries []ViewerEntry, viewerIdx int) {
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }
	cardKey := fmt.Sprintf("done-%s-%d", job.ID, outIdx)
	src := resolveThumb(state, path)
	missing := state.Thumbs[path].Status == ThumbMissing

	ui.Column(c.Key(cardKey)).Width(studioCardW).Gap(8).Children(func() {
		if missing {
			ui.Column(c.Key("missing")).Size(studioCardW, studioCardW).Radius(p.RadiusLg).Border(1, p.Border).Background(p.Muted).Center().Padding(12).Gap(6).Children(func() {
				ui.Icon(c, icons.Image).FontSize(22).TextColor(p.MutedForeground)
				ui.Text(c, t("image_studio_file_missing")).FontSize(12).TextColor(p.MutedForeground)
			})
		} else {
			kit.Image(c.Key("img"), src, kit.ImageProps{
				Size:    studioCardW,
				Loading: src == nil,
				Alt:     job.Spec.Prompt,
				OnOpen: func() {
					state.Viewer.Show(allEntries, viewerIdx)
				},
			})
		}
		renderJobCaption(c, state, job, func() {
			menuID := "studio-card-menu-" + cardKey
			kit.MenuButton(c.Key("menu"), menuID, func(b ui.Element) {
				kit.IconLook(c, b, icons.Ellipsis, t("more"), "", 24, false)
			}, func(m *kit.Menu) {
				if !missing && state.SaveImageAs != nil {
					if m.Item(t("image_studio_save_as")).Icon(icons.Save).Chosen() {
						state.SaveImageAs(path)
					}
				}
				if !missing && state.RevealImage != nil {
					if m.Item(t("image_studio_reveal")).Icon(icons.FolderOpen).Chosen() {
						state.RevealImage(path)
					}
				}
				if m.Item(t("image_studio_copy_prompt")).Icon(icons.Copy).Chosen() {
					c.WriteClipboard(job.Spec.Prompt)
					kit.Copied(c)
				}
				if m.Item(t("image_studio_edit_prompt")).Icon(icons.Pencil).Chosen() {
					state.Prompt = job.Spec.Prompt
				}
				if state.RetryJob != nil {
					if m.Item(t("image_studio_rerun")).Icon(icons.Reload).Chosen() {
						state.RetryJob(job.ID)
					}
				}
				m.Separator()
				if state.RemoveJob != nil {
					if m.Item(t("image_studio_remove")).Icon(icons.Trash).Danger().Chosen() {
						state.RemoveJob(job.ID)
					}
				}
			}, kit.MenuOptions{At: ui.AnchorBottomEnd, Self: ui.AnchorTopEnd})
		})
	})
}

func renderRunningCard(c *ui.Context, state *State, job *core.Job, elapsedSec int64) {
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }
	if elapsedSec < 0 {
		elapsedSec = 0
	}
	elapsed := fmt.Sprintf("%d:%02d", elapsedSec/60, elapsedSec%60)

	detail := t("image_studio_running_direct")
	if job.TaskID != "" {
		detail = t("image_studio_running_task")
	} else if job.GroupName != "" {
		detail = job.GroupName
	} else if job.Endpoint != "" {
		detail = job.Endpoint
	}

	ui.Column(c.Key("running-"+job.ID)).Width(studioCardW).Gap(8).Children(func() {
		ui.Column(c.Key("box")).Size(studioCardW, studioCardW).Radius(p.RadiusLg).Border(1, p.Border).Background(p.Surface).Center().Padding(16).Gap(10).Children(func() {
			if state.CancelJob != nil {
				kit.IconButton(c.Key("cancel"), icons.Close, t("image_studio_cancel"), 24, false).
					Absolute().Top(8).Right(8).
					OnClick(func() { state.CancelJob(job.ID) })
			}
			kit.Loader(c, 22)
			ui.Text(c, t("image_studio_drawing", "time", elapsed)).FontSize(13).FontWeight(500).TextColor(p.Foreground)
			ui.Text(c, detail).FontSize(12).SingleLine().TextColor(p.MutedForeground)
			if state.CancelJob != nil {
				kit.Secondary(c.Key("cancel-btn"), t("image_studio_cancel")).OnClick(func() {
					state.CancelJob(job.ID)
				})
			}
		})
		renderJobCaption(c, state, job, nil)
	})
}

func renderFailedCard(c *ui.Context, state *State, job *core.Job) {
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }

	cancelled := job.Status == core.JobCancelled || (job.Error != nil && job.Error.Kind == core.ErrCancelled)
	title := t("image_studio_error_other")
	detail := ""
	needsSettings := false
	if job.Error != nil {
		title = t(job.Error.MessageKey())
		detail = strings.TrimSpace(job.Error.Message)
		switch job.Error.Kind {
		case core.ErrUnauthorized, core.ErrNoImagePermission, core.ErrWrongPlatform, core.ErrModelUnavailable:
			needsSettings = true
		}
	} else if cancelled {
		title = t("image_studio_error_cancelled")
	}

	accent := p.Danger
	icon := icons.TriangleAlert
	if cancelled {
		accent = p.MutedForeground
		icon = icons.CircleX
	}

	ui.Column(c.Key("failed-"+job.ID)).Width(studioCardW).Gap(8).Children(func() {
		ui.Column(c.Key("box")).Size(studioCardW, studioCardW).Radius(p.RadiusLg).Border(1, accent.Alpha(0.35)).
			Background(p.Background.Mix(accent, 0.05)).Padding(14).Gap(8).Children(func() {
			ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
				ui.Icon(c, icon).FontSize(16).TextColor(accent).Shrink(0)
				ui.Text(c, title).FontSize(13).FontWeight(500).TextColor(p.Foreground).Grow(1).Shrink(1).MinWidth(0)
			})
			if detail != "" && !cancelled {
				ui.Text(c, detail).FontSize(12).TextColor(p.MutedForeground).Grow(1).Shrink(1).MinHeight(0)
			} else {
				ui.Spacer(c)
			}
			ui.Row(c.Key("actions")).Gap(6).Wrap().Children(func() {
				if state.RetryJob != nil {
					kit.Secondary(c.Key("retry"), t("image_studio_retry")).OnClick(func() {
						state.RetryJob(job.ID)
					})
				}
				if needsSettings && state.OpenSettings != nil {
					kit.TextAction(c.Key("settings"), t("image_studio_open_settings"), false).OnClick(state.OpenSettings)
				}
				if state.RemoveJob != nil {
					kit.IconButton(c.Key("remove"), icons.Trash, t("image_studio_remove"), 28, false).OnClick(func() {
						state.RemoveJob(job.ID)
					})
				}
			})
		})
		renderJobCaption(c, state, job, nil)
	})
}

func renderJobCaption(c *ui.Context, state *State, job *core.Job, trailing func()) {
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }

	var meta []string
	if job.Spec.Model != "" {
		meta = append(meta, job.Spec.Model)
	}
	if job.Spec.Size != "" {
		meta = append(meta, sizeLabel(c, job.Spec.Size))
	}
	if job.Spec.Channel == core.ChannelCustom {
		meta = append(meta, t("image_studio_channel_custom"))
	} else {
		meta = append(meta, t("image_studio_channel_cheaprouter"))
	}
	if job.EstimateUSD != nil {
		meta = append(meta, formatUSD(*job.EstimateUSD))
	}

	ui.Column(c.Key("caption")).FillWidth().Gap(2).Children(func() {
		ui.Row(c).FillWidth().Gap(4).AlignItems(ui.Center).Children(func() {
			ui.Text(c, job.Spec.Prompt).FontSize(13).SingleLine().TextColor(p.Foreground).Grow(1).Shrink(1).MinWidth(0)
			if trailing != nil {
				trailing()
			}
		})
		ui.Text(c, strings.Join(meta, " · ")).FontSize(11).SingleLine().TextColor(p.MutedForeground)
	})
}

func renderComposer(c *ui.Context, state *State) {
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }

	ui.Row(c.Key("composer-dock")).FillWidth().Shrink(0).BorderWidth(1, 0, 0, 0).BorderColor(p.Border).Background(p.Surface).Children(func() {
		ui.Column(c).FillWidth().MaxWidth(studioMaxWidth).Margin(0, ui.Auto).Padding(14, 24, 16, 24).Gap(10).Children(func() {
			area := ui.TextArea(c.Key("prompt"), &state.Prompt).
				Label(t("image_studio")).
				Placeholder(t("image_studio_prompt_placeholder")).
				FillWidth().
				MinHeight(64).
				MaxHeight(140)
			if area.Shortcut(ui.Cmd, ui.KeyEnter) && !c.ShortcutRepeated() && state.CanSubmit() && state.Submit != nil {
				state.Submit()
			}

			ui.Row(c.Key("controls")).FillWidth().Gap(8).AlignItems(ui.Center).Wrap().Children(func() {
				// Model control
				if state.EffectiveChannel() == core.ChannelCustom {
					modelLabel := t("settings_image_api_status_empty")
					if state.Custom.Configured {
						modelLabel = state.Custom.Model
						if host := core.DisplayHost(state.Custom.BaseURL); host != "" {
							modelLabel += " · " + host
						}
					}
					kit.Secondary(c.Key("custom-model"), modelLabel).OnClick(func() {
						if state.OpenSettings != nil {
							state.OpenSettings()
						}
					})
				} else {
					chosen := state.EffectiveModel()
					options := make([]kit.SelectOption, 0, len(state.CheapModels))
					for _, m := range state.CheapModels {
						label := m.ID
						if m.DisplayName != "" && !strings.EqualFold(m.DisplayName, m.ID) {
							label = m.DisplayName + " (" + m.ID + ")"
						}
						options = append(options, kit.SelectOption{Value: m.ID, Label: label})
					}
					_, changed := kit.SelectField(c.Key("model"), "studio-model", &chosen, kit.SelectProps{
						Label:       t("image_studio_model"),
						Placeholder: t("image_studio_model"),
						Options:     options,
						Disabled:    len(options) == 0,
					})
					if changed {
						state.Model = chosen
						if state.OnPrefsChanged != nil {
							state.OnPrefsChanged()
						}
					}
				}

				// Size control
				sizeVal := state.EffectiveSize()
				sizeOptsRaw := core.SizeOptions(state.EffectiveModel())
				sizeOpts := make([]kit.SelectOption, len(sizeOptsRaw))
				for i, opt := range sizeOptsRaw {
					sizeOpts[i] = kit.SelectOption{Value: opt, Label: sizeLabel(c, opt)}
				}
				if _, changed := kit.SelectField(c.Key("size"), "studio-size", &sizeVal, kit.SelectProps{
					Label:   t("image_studio_size"),
					Options: sizeOpts,
				}); changed {
					state.Size = sizeVal
					if state.OnPrefsChanged != nil {
						state.OnPrefsChanged()
					}
				}

				// Quality control (only when model supports quality)
				if core.SupportsQuality(state.EffectiveModel()) {
					qualVal := state.EffectiveQuality()
					qualRaw := core.QualityOptions(state.EffectiveModel())
					qualOpts := make([]kit.SelectOption, len(qualRaw))
					for i, opt := range qualRaw {
						qualOpts[i] = kit.SelectOption{Value: opt, Label: qualityLabel(c, opt)}
					}
					if _, changed := kit.SelectField(c.Key("quality"), "studio-quality", &qualVal, kit.SelectProps{
						Label:   t("image_studio_quality"),
						Options: qualOpts,
					}); changed {
						state.Quality = qualVal
						if state.OnPrefsChanged != nil {
							state.OnPrefsChanged()
						}
					}
				}

				// Count control (1..4)
				countVal := strconv.Itoa(state.EffectiveCount())
				countOpts := make([]kit.SelectOption, core.MaxCount)
				for i := 1; i <= core.MaxCount; i++ {
					countOpts[i-1] = kit.SelectOption{
						Value: strconv.Itoa(i),
						Label: t("image_studio_count_n", "count", i),
					}
				}
				if _, changed := kit.SelectField(c.Key("count"), "studio-count", &countVal, kit.SelectProps{
					Label:   t("image_studio_count"),
					Options: countOpts,
				}); changed {
					if parsed, err := strconv.Atoi(countVal); err == nil {
						state.Count = parsed
						if state.OnPrefsChanged != nil {
							state.OnPrefsChanged()
						}
					}
				}

				ui.Spacer(c)

				if est, ok := state.EstimateUSD(); ok {
					kit.Tag(c, "~"+formatUSD(est))
				}

				kit.Primary(c.Key("generate"), t("image_studio_generate"), !state.CanSubmit()).OnClick(func() {
					if state.Submit != nil {
						state.Submit()
					}
				})
			})
		})
	})
}

func resolveThumb(state *State, path string) ui.ImageSource {
	if state == nil || path == "" {
		return nil
	}
	entry, ok := state.Thumbs[path]
	if ok && entry.Status == ThumbReady {
		return entry.Image
	}
	if (!ok || entry.Status == ThumbIdle) && state.RequestThumb != nil {
		state.Thumbs[path] = Thumb{Status: ThumbLoading}
		state.RequestThumb(path)
	}
	return nil
}

func sizeLabel(c *ui.Context, size string) string {
	if strings.EqualFold(strings.TrimSpace(size), "auto") {
		return i18n.T(c, "image_studio_size_auto")
	}
	return strings.ReplaceAll(size, "x", "×")
}

func qualityLabel(c *ui.Context, quality string) string {
	switch strings.ToLower(strings.TrimSpace(quality)) {
	case "low":
		return i18n.T(c, "image_studio_quality_low")
	case "medium":
		return i18n.T(c, "image_studio_quality_medium")
	case "high":
		return i18n.T(c, "image_studio_quality_high")
	case "standard":
		return i18n.T(c, "image_studio_quality_standard")
	case "hd":
		return i18n.T(c, "image_studio_quality_hd")
	default:
		return i18n.T(c, "image_studio_quality_auto")
	}
}

func formatUSD(v float64) string {
	if v < 0.1 {
		return fmt.Sprintf("$%.3f", v)
	}
	return fmt.Sprintf("$%.2f", v)
}
