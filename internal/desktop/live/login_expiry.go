package live

import "sync"

// Each window has its own Service and UI queue. Credential loss belongs to
// the profile, but dismissing the banner is a choice for the entire process.
// Neither belongs in preferences or in the credential file.
var desktopExpiry loginExpiry

type loginExpiry struct {
	mu        sync.Mutex
	expired   map[string]bool
	dismissed bool
	windows   map[*func()]string
}

func (e *loginExpiry) watch(profile string, wake func()) func() {
	e.mu.Lock()
	if e.windows == nil {
		e.windows = make(map[*func()]string)
	}
	e.windows[&wake] = profile
	e.mu.Unlock()
	return func() {
		e.mu.Lock()
		delete(e.windows, &wake)
		e.mu.Unlock()
	}
}

func (e *loginExpiry) snapshot(profile string) (expired, dismissed bool) {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.expired[profile], e.dismissed
}

func (e *loginExpiry) set(profile string, expired bool) {
	e.mu.Lock()
	if e.expired[profile] == expired {
		e.mu.Unlock()
		return
	}
	if expired {
		if e.expired == nil {
			e.expired = make(map[string]bool)
		}
		e.expired[profile] = true
	} else {
		delete(e.expired, profile)
	}
	callbacks := e.callbacksLocked(profile)
	e.mu.Unlock()
	for _, f := range callbacks {
		f()
	}
}

func (e *loginExpiry) dismiss() {
	e.mu.Lock()
	if e.dismissed {
		e.mu.Unlock()
		return
	}
	e.dismissed = true
	callbacks := e.callbacksLocked("")
	e.mu.Unlock()
	for _, f := range callbacks {
		f()
	}
}

func (e *loginExpiry) callbacksLocked(profile string) []func() {
	var callbacks []func()
	for f, p := range e.windows {
		if profile == "" || profile == p {
			callbacks = append(callbacks, *f)
		}
	}
	return callbacks
}
