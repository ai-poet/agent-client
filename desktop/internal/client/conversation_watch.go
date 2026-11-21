package client

import (
	"context"
	"errors"
	"io"
	"slices"
	"sync"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/link"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

type ConversationState struct {
	Snapshot                                  protocol.ConversationSnapshot
	Connected, Loading, LoadingOlder, Removed bool
	Err, OlderErr                             error
	// StatisticsErr is the fault of the Node's last statistics refresh, until
	// statistics or a snapshot arrive.
	StatisticsErr error
	// InventoryVersion changes when the Node's canonical asset inventory may
	// have changed. It is a local cache key, not the structural page revision.
	InventoryVersion uint64 `json:"-"`
	viewSnapshot     *byte
}

type conversationWatch struct {
	session         protocol.SessionID
	projection      *ConversationProjection
	viewSnapshot    *byte
	state           ConversationState
	ctx             context.Context
	generationCtx   context.Context
	cancel          context.CancelFunc
	done            chan struct{}
	refresh, signal chan struct{}
	callbacks       map[uint64]func()
	next            uint64
	extent          conversationExtent
	known           bool
}

const conversationBookmarkLimit = 64

type conversationBookmark struct {
	session protocol.SessionID
	extent  conversationExtent
	known   bool
}

func (w *conversationWatch) discardHistory() {
	w.projection.snapshot = nil
	w.viewSnapshot = nil
	w.projection.liveStart = nil
	w.projection.statisticsErr = nil
	w.projection.hidden, w.projection.revealed = nil, nil
	w.extent = conversationExtent{}
	w.state.LoadingOlder, w.state.OlderErr = false, nil
	w.state.InventoryVersion = 0
}

// A second reconnect during automatic paging must recover the original range,
// not just the prefix the interrupted generation had time to load.
func (w *conversationWatch) rememberExtent() {
	if snapshot := w.projection.snapshot; snapshot != nil && !w.extent.needsMore(*snapshot) {
		w.extent = conversationExtentOf(*snapshot)
	}
}

func (c *Client) Conversation(session protocol.SessionID) (ConversationState, bool) {
	return c.readConversationState(session, nil)
}

// ConversationView returns a detached read-only snapshot, which may share the
// previous view's payload. Callers that need a mutable copy use Conversation.
func (c *Client) ConversationView(session protocol.SessionID, previous ConversationState) (ConversationState, bool) {
	return c.readConversationState(session, &previous)
}

// SameViewSnapshot compares complete read-only payloads, not watch status,
// history revision or stream sequence alone. Paging can change the payload
// without changing the latter two.
func (s ConversationState) SameViewSnapshot(other ConversationState) bool {
	return s.viewSnapshot != nil && s.viewSnapshot == other.viewSnapshot
}

func (c *Client) readConversationState(session protocol.SessionID, previous *ConversationState) (ConversationState, bool) {
	c.conversationMu.Lock()
	defer c.conversationMu.Unlock()
	w := c.conversations[session]
	if w == nil {
		return ConversationState{}, false
	}
	s := w.state
	s.StatisticsErr = w.projection.statisticsErr
	if snapshot := w.projection.snapshot; snapshot != nil {
		if previous != nil {
			if w.viewSnapshot == nil {
				w.viewSnapshot = new(byte)
			}
			s.viewSnapshot = w.viewSnapshot
			if s.SameViewSnapshot(*previous) {
				s.Snapshot = previous.Snapshot
				return s, true
			}
		}
		s.Snapshot = copyValue(*snapshot)
	}
	return s, true
}

// WatchConversation shares a single subscription and reduction owner per session.
func (c *Client) WatchConversation(ctx context.Context, session protocol.SessionID, changed func()) (func(), error) {
	c.lifecycle.Lock()
	defer c.lifecycle.Unlock()
	if c.closed {
		return nil, fault(protocol.ErrorCodeUnavailable, "Client is closed")
	}
	c.conversationMu.Lock()
	if c.conversations == nil {
		c.conversations = map[protocol.SessionID]*conversationWatch{}
	}
	w := c.conversations[session]
	if w == nil || w.ctx.Err() != nil {
		parent := c.context
		if parent == nil {
			parent = ctx
		}
		owner, cancel := context.WithCancel(parent)
		next := &conversationWatch{session: session, projection: NewConversationProjection(c.Target(), session, 0), ctx: owner, cancel: cancel, done: make(chan struct{}), refresh: make(chan struct{}, 1), signal: make(chan struct{}), callbacks: map[uint64]func(){}, state: ConversationState{Loading: true}}
		if w != nil {
			next.extent = w.extent
			w.discardHistory()
		}
		// A bookmark describes how far to page after authorization; it is not
		// a display snapshot that can be delivered by a new subscription.
		bookmarks := c.conversationBookmarks
		next.known = c.sessionKnown(session)
		if at := slices.IndexFunc(bookmarks, func(b conversationBookmark) bool { return b.session == session }); at >= 0 {
			next.extent = bookmarks[at].extent
			next.known = next.known || bookmarks[at].known
			c.conversationBookmarks = slices.Delete(bookmarks, at, at+1)
		}
		c.conversations[session] = next
		if w != nil {
			c.pruneConversationDetails()
		}
		w = next
		c.conversationOwners.Add(1)
		go c.observeConversation(w)
	}
	w.next++
	id := w.next
	w.callbacks[id] = changed
	c.conversationMu.Unlock()
	var once sync.Once
	release := func() {
		once.Do(func() {
			c.conversationMu.Lock()
			delete(w.callbacks, id)
			if len(w.callbacks) == 0 {
				w.cancel()
				if c.conversations[session] == w {
					if w.extent != (conversationExtent{}) {
						if len(c.conversationBookmarks) == conversationBookmarkLimit {
							c.conversationBookmarks = slices.Delete(c.conversationBookmarks, 0, 1)
						}
						c.conversationBookmarks = append(c.conversationBookmarks, conversationBookmark{session, w.extent, w.known})
					}
					delete(c.conversations, session)
					w.discardHistory()
					c.pruneConversationDetails()
				}
			}
			c.conversationMu.Unlock()
		})
	}
	stop := context.AfterFunc(ctx, release)
	return func() { stop(); release() }, nil
}

func (c *Client) RefreshConversation(ctx context.Context, session protocol.SessionID) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	c.conversationMu.Lock()
	w := c.conversations[session]
	c.conversationMu.Unlock()
	if w == nil || w.ctx.Err() != nil {
		return fault(protocol.ErrorCodeUnavailable, "conversation observation is unavailable")
	}
	select {
	case w.refresh <- struct{}{}:
	default:
	}
	return nil
}

func (c *Client) conversationChanged(w *conversationWatch) {
	c.conversationMu.Lock()
	if c.conversations[w.session] != w {
		c.conversationMu.Unlock()
		return
	}
	callbacks := make([]func(), 0, len(w.callbacks))
	for _, f := range w.callbacks {
		if f != nil {
			callbacks = append(callbacks, f)
		}
	}
	close(w.signal)
	w.signal = make(chan struct{})
	c.conversationMu.Unlock()
	for _, f := range callbacks {
		f()
	}
}

func (c *Client) closeConversations() {
	c.conversationMu.Lock()
	var owners []*conversationWatch
	for _, w := range c.conversations {
		w.cancel()
		w.discardHistory()
		owners = append(owners, w)
	}
	c.conversations, c.conversationBookmarks = nil, nil
	c.pruneConversationDetails()
	c.conversationMu.Unlock()
	for _, w := range owners {
		<-w.done
	}
	c.conversationOwners.Wait()
}

// A resource read miss is not removal; the authoritative Node snapshot confirms it.
func (c *Client) reconcileConversations() {
	c.conversationMu.Lock()
	c.conversationBookmarks = slices.DeleteFunc(c.conversationBookmarks, func(b conversationBookmark) bool {
		return b.known && !c.sessionKnown(b.session)
	})
	var removed []*conversationWatch
	for id, w := range c.conversations {
		if c.sessionKnown(id) {
			w.known = true
		} else if w.known && !w.state.Removed {
			w.discardHistory()
			w.state.Removed = true
			w.state.Connected = false
			w.state.Loading = false
			w.state.Err = fault(protocol.ErrorCodeNotFound, "conversation was removed")
			w.cancel()
			if len(w.callbacks) == 0 {
				delete(c.conversations, id)
				continue
			}
			removed = append(removed, w)
		}
	}
	c.pruneConversationDetails()
	c.conversationMu.Unlock()
	for _, w := range removed {
		c.conversationChanged(w)
	}
}

type conversationIncoming struct {
	update protocol.Update
	err    error
}

func (c *Client) observeConversation(w *conversationWatch) {
	defer c.conversationOwners.Done()
	defer close(w.done)
	defer func() {
		c.conversationMu.Lock()
		w.state.Connected = false
		w.state.Loading = false
		w.state.LoadingOlder = false
		c.conversationMu.Unlock()
		c.conversationChanged(w)
	}()
	delay := 250 * time.Millisecond
	var outage time.Time
	for w.ctx.Err() == nil {
		ctx, cancel := context.WithCancel(w.ctx)
		c.conversationMu.Lock()
		w.projection.generation++
		generation := w.projection.generation
		w.projection.recovering = true
		w.generationCtx = ctx
		desired := w.extent
		c.conversationMu.Unlock()
		wake := c.retried()
		sub, err := c.transport.Subscribe(ctx, protocol.Topic{Kind: "conversation", Data: w.session})
		if err == nil {
			err = c.consumeConversation(ctx, cancel, w, sub, generation, desired)
		}
		cancel()
		if err == nil {
			delay = 250 * time.Millisecond
			outage = time.Time{}
			continue
		}
		if w.ctx.Err() != nil {
			return
		}
		if !retryConversation(err) {
			c.conversationMu.Lock()
			switch faultCode(err) {
			case protocol.ErrorCodePermissionDenied, protocol.ErrorCodeNotFound:
				w.discardHistory()
				c.pruneConversationDetails()
			}
			w.state.Err = err
			w.state.Connected = false
			w.state.Loading = false
			c.conversationMu.Unlock()
			c.conversationChanged(w)
			select {
			case <-w.ctx.Done():
				return
			case <-w.refresh:
				continue
			}
		}
		if outage.IsZero() {
			outage = time.Now()
		}
		if time.Since(outage) >= 5*time.Second {
			c.conversationMu.Lock()
			w.state.Err = err
			w.state.Connected = faultCode(err) == protocol.ErrorCodeBusy && w.state.Connected
			c.conversationMu.Unlock()
			c.conversationChanged(w)
		}
		timer := time.NewTimer(delay)
		select {
		case <-w.ctx.Done():
			timer.Stop()
			return
		case <-w.refresh:
			timer.Stop()
		case <-wake:
			timer.Stop()
			delay = 250 * time.Millisecond
			continue
		case <-timer.C:
		}
		delay = min(delay*2, 5*time.Second)
	}
}

func retryConversation(err error) bool {
	return transient(err) || errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, context.Canceled)
}

func (c *Client) consumeConversation(ctx context.Context, cancel context.CancelFunc, w *conversationWatch, sub link.Subscription, generation uint64, desired conversationExtent) error {
	stream := make(chan conversationIncoming, 1)
	joined := make(chan struct{})
	go func() {
		defer close(joined)
		for {
			update, err := sub.Next(ctx)
			select {
			case stream <- conversationIncoming{update, err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	var restoreDone chan struct{}
	defer func() {
		cancel()
		sub.Close()
		<-joined
		if restoreDone != nil {
			<-restoreDone
		}
	}()
	restored := false
	for {
		select {
		case <-w.ctx.Done():
			return w.ctx.Err()
		case <-w.refresh:
			return nil
		case incoming := <-stream:
			if incoming.err != nil {
				return incoming.err
			}
			applied, err := c.applyConversationUpdate(w, generation, incoming.update)
			if err != nil {
				return err
			}
			if applied == Recover {
				return nil
			}
			if applied == Ignored {
				continue
			}
			c.conversationChanged(w)
			if !restored {
				restored = true
				restoreDone = make(chan struct{})
				go func() { defer close(restoreDone); c.restoreConversation(ctx, w, generation, desired) }()
			}
		}
	}
}

func (c *Client) applyConversationUpdate(w *conversationWatch, generation uint64, update protocol.Update) (Apply, error) {
	c.conversationMu.Lock()
	defer c.conversationMu.Unlock()
	if c.conversations[w.session] != w || w.state.Removed || w.ctx.Err() != nil {
		return Ignored, context.Canceled
	}
	applied, err := w.projection.Apply(generation, update)
	if err == nil && applied == Applied {
		w.viewSnapshot = nil
		if changesConversationInventory(update) {
			c.conversationInventory++
			w.state.InventoryVersion = c.conversationInventory
		}
		w.state.Loading = false
		w.state.Connected = true
		w.state.Err = nil
		w.rememberExtent()
		c.pruneConversationDetails()
	}
	return applied, err
}

// The inventory reads canonical records, including hidden ones outside the
// fitted page. Only known non-inventory frames can reuse a previous read.
func changesConversationInventory(update protocol.Update) bool {
	if update.Kind == "conversation_frame" {
		frame, _ := payload[protocol.ConversationFrame](update.Data) // already accepted by Apply
		switch frame.Change.Kind {
		case "delta", "statistics", "statistics_failed", "run", "queue", "approval", "question", "child", "background":
			return false
		}
	}
	return true
}

func (c *Client) restoreConversation(ctx context.Context, w *conversationWatch, generation uint64, desired conversationExtent) {
	turnLimit := turnHistoryPageLimit
	if desired.all {
		// The Node also bounds each page by bytes. Leave interactive reads
		// and recovery to a partial anchor at the smaller entry limit.
		turnLimit = fullHistoryRestoreLimit
	}
	for ctx.Err() == nil {
		c.conversationMu.Lock()
		valid := c.conversations[w.session] == w && w.projection.generation == generation
		needed := valid && w.projection.snapshot != nil && desired.needsMore(*w.projection.snapshot)
		loading, signal := w.state.LoadingOlder, w.signal
		c.conversationMu.Unlock()
		if !needed {
			return
		}
		if loading {
			select {
			case <-ctx.Done():
				return
			case <-signal:
				continue
			}
		}
		if err := c.loadOlderConversation(ctx, w.session, turnLimit, w, generation); err != nil {
			return
		}
	}
}
