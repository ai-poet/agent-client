package live

import (
	"fmt"
	"strings"

	"github.com/ai-poet/agent-client/internal/desktop/settings"
	"github.com/ai-poet/agent-client/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// Settings › Media models: the model the Node Settings shows uses for
// vision and for generating images, video and speech. The assignments and
// the models offered are read again whenever the providers or the
// assignments change, and a choice saves all four at the revision read.

// mediaState is what the media rows were read for and the bindings their
// option keys stand for.
type mediaState struct {
	binding   uint64 // increments when these Settings pages leave the Node
	attempted mediaReadKey
	accepted  mediaReadKey
	wanted    mediaReadKey
	reading   bool
	bindings  map[string]protocol.MediaBinding
}

type mediaReadKey struct {
	observation uint64
	fingerprint string
	revision    uint64
}

// followMedia reads on a changed snapshot, reconnect, or explicit Refresh.
// A failed pair is attempted but not accepted; frames do not retry it.
func (r *Runtime) followMedia(snapshot protocol.Snapshot) {
	if !r.showsSettings() {
		return
	}
	state := &r.media
	state.wanted = mediaReadKey{observation: r.controller.ObservationGeneration(), revision: snapshot.MediaSettings.Revision}
	if state.wanted.observation == 0 {
		return
	}
	var providers strings.Builder
	for _, provider := range snapshot.Providers {
		fmt.Fprintf(&providers, " %s/%d", provider.ID, provider.Revision)
	}
	state.wanted.fingerprint = providers.String()
	r.readWantedMedia()
}

func (r *Runtime) readWantedMedia() {
	state := &r.media
	if !state.reading && state.wanted.observation != 0 && state.wanted != state.attempted && state.wanted != state.accepted {
		r.readMedia()
	}
}

// readMedia projects only a validated pair for the current target. A newer
// snapshot arriving during either read gets its own pair when this one ends.
func (r *Runtime) readMedia() {
	state := &r.media
	binding, wanted := state.binding, state.wanted
	state.reading = r.async(func() {
		saved, err := r.controller.Do(r.ctx, protocol.Command{Kind: "read_media_settings"})
		value, valid := detail[protocol.MediaSettings](saved.Data)
		if err == nil && (saved.Kind != "media_settings" || !valid) {
			err = fmt.Errorf("unexpected media settings response")
		}
		var listed protocol.Output
		if err == nil {
			listed, err = r.controller.Do(r.ctx, protocol.Command{Kind: "list_media_models"})
		}
		candidates, valid := detail[[]protocol.MediaCandidate](listed.Data)
		if err == nil && (listed.Kind != "media_models" || !valid) {
			err = fmt.Errorf("unexpected media models response")
		}
		r.enqueue(func(c *ui.Context) {
			if !r.showsSettings() || binding != state.binding {
				return
			}
			state.reading = false
			if wanted.observation != r.controller.ObservationGeneration() {
				r.followMedia(r.snapshot())
				return
			}
			defer r.readWantedMedia()
			if wanted != state.wanted {
				return
			}
			if err != nil {
				r.reportKeys(c, err, "media_load_failed", "media_load_failed", "media_load_failed")
			} else if value.Revision >= max(wanted.revision, r.App.Settings.Media.Revision) {
				r.projectMedia(value, candidates, true)
				state.accepted = wanted
			}
		})
	})
	if state.reading {
		state.attempted = wanted
	}
}

// projectMedia shows the assignments the Node holds. A draft that differs
// from what was saved (its save failed, conflicted or is in doubt) stays
// the rows' choice; listed replaces the models offered.
func (r *Runtime) projectMedia(value protocol.MediaSettings, candidates []protocol.MediaCandidate, listed bool) {
	m := r.App.Settings
	state := &r.media
	if listed {
		// A draft's model that is no longer offered stays its row's choice.
		kept := map[string]viewmodel.MediaOption{}
		for _, option := range m.MediaOptions {
			for _, key := range m.Media.Draft {
				if key == option.Key {
					kept[key] = viewmodel.MediaOption{Key: key, Label: option.Label}
				}
			}
		}
		previous := state.bindings
		defer func() {
			for key, option := range kept {
				if _, offered := state.bindings[key]; !offered {
					state.bindings[key] = previous[key]
					m.MediaOptions = append(m.MediaOptions, option)
				}
			}
		}()
		state.bindings = map[string]protocol.MediaBinding{}
		m.MediaOptions = nil
		for _, candidate := range candidates {
			binding := protocol.MediaBinding{Provider: candidate.Provider, Model: candidate.Model}
			option := viewmodel.MediaOption{Key: mediaKey(binding), Label: candidate.ProviderName + " · " + candidate.Model}
			for _, kind := range candidate.Kinds {
				option.Kinds = append(option.Kinds, string(kind))
			}
			state.bindings[option.Key] = binding
			m.MediaOptions = append(m.MediaOptions, option)
		}
	}
	var bound [4]string
	m.MediaUnavailable = [4]bool{}
	for i, kind := range viewmodel.MediaKinds {
		binding := value.Bindings[protocol.MediaKind(kind)]
		if binding == nil {
			continue
		}
		bound[i] = mediaKey(*binding)
		if _, offered := state.bindings[bound[i]]; !offered {
			// A saved model that is no longer offered stays its row's choice,
			// named by its provider where the Node still has it.
			label := binding.Model
			if provider, exists := r.provider(binding.Provider.String()); exists {
				label = provider.Name + " · " + binding.Model
			}
			state.bindings[bound[i]] = *binding
			m.MediaOptions = append(m.MediaOptions, viewmodel.MediaOption{Key: bound[i], Label: label})
		}
	}
	for _, kind := range value.Unavailable {
		for i, name := range viewmodel.MediaKinds {
			m.MediaUnavailable[i] = m.MediaUnavailable[i] || name == string(kind)
		}
	}
	field := &m.Media
	pristine := field.Draft == field.Saved
	field.Saved, field.Revision = bound, value.Revision
	if pristine {
		field.Draft = bound
	}
}

func mediaKey(binding protocol.MediaBinding) string {
	return modelKey(binding.Provider, binding.Model)
}

// saveMedia sends the rows' choices at the revision read. Failure, conflict
// and an unknown outcome are toasts and keep the choices as the draft; the
// assignments are then read again, so that the next choice saves at the
// Node's revision.
func (r *Runtime) saveMedia() {
	field := &r.App.Settings.Media
	observation, binding := r.controller.ObservationGeneration(), r.media.binding
	if !r.showsSettings() || observation == 0 || field.Pending {
		return
	}
	request := protocol.MediaSettings{Revision: field.Revision, Bindings: map[protocol.MediaKind]*protocol.MediaBinding{}}
	for i, kind := range viewmodel.MediaKinds {
		if binding, chosen := r.media.bindings[field.Draft[i]]; chosen {
			request.Bindings[protocol.MediaKind(kind)] = &binding
		}
	}
	field.Pending = true
	r.saveSetting(protocol.Command{Kind: "save_media_settings", Data: request}, settings.MediaMessages, func(output protocol.Output, ok bool) {
		if !r.showsSettings() || binding != r.media.binding {
			return
		}
		field.Pending = false
		if observation != r.controller.ObservationGeneration() {
			return
		}
		value, valid := detail[protocol.MediaSettings](output.Data)
		if !ok || !valid || output.Kind != "media_settings" {
			r.media.accepted, r.media.attempted = mediaReadKey{}, mediaReadKey{}
			r.followMedia(r.snapshot())
			return
		}
		if value.Revision >= field.Revision {
			field.Draft = field.Saved
			r.projectMedia(value, nil, false)
		}
	})
}
