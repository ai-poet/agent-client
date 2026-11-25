package engine

import (
	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/trace"
)

func (r *Runtime) scopeTrace(call *fantasy.Call, language fantasy.LanguageModel, x *execution, binding model.Binding, purpose canonical.Purpose) *trace.Scope {
	if !r.db.TraceEnabled() {
		return nil
	}
	provider, ok := model.TraceProvider(language, binding)
	if !ok {
		return nil
	}
	if call.ProviderOptions == nil {
		call.ProviderOptions = fantasy.ProviderOptions{}
	}
	scope := &trace.Scope{Profile: r.profile, Session: x.inv.Session.ID.String(), Turn: x.inv.Run.Turn.String(), Purpose: string(purpose), Provider: provider, Enabled: r.db.TraceEnabled}
	call.ProviderOptions[trace.ProviderKey] = scope
	return scope
}
