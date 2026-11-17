package daemon

import (
	"context"
	"fmt"
)

// TaskState is the lightweight daemon-owned project/session catalog.
type TaskState struct {
	Projects        []Project
	Sessions        []AgentSession
	DefaultCwd      string
	ProjectlessRoot *string
}

// SessionRuntime identifies a live provider runtime returned by attachSession.
type SessionRuntime struct {
	RuntimeID     string
	SupportsSteer bool
}

func (c *Client) LoadTaskState(ctx context.Context) (TaskState, error) {
	payload, err := c.Request(ctx, NilUUID, NilUUID, map[string]any{"type": "loadTaskState"})
	if err != nil {
		return TaskState{}, err
	}
	if payload.Type != "taskState" {
		return TaskState{}, unexpectedPayload("taskState", payload.Type)
	}
	return TaskState{
		Projects:        payload.Projects,
		Sessions:        payload.Sessions,
		DefaultCwd:      payload.DefaultCwd,
		ProjectlessRoot: payload.ProjectlessRoot,
	}, nil
}

func (c *Client) HydrateSession(ctx context.Context, sessionID string) (*AgentSession, error) {
	payload, err := c.Request(ctx, NilUUID, NilUUID, map[string]any{
		"type": "hydrateSession", "sessionId": sessionID,
	})
	if err != nil {
		return nil, err
	}
	if payload.Type != "session" {
		return nil, unexpectedPayload("session", payload.Type)
	}
	return payload.Session, nil
}

func (c *Client) AttachSession(ctx context.Context, sessionID string) (*SessionRuntime, error) {
	payload, err := c.Request(ctx, sessionID, NilUUID, map[string]any{"type": "attachSession"})
	if err != nil {
		return nil, err
	}
	if payload.Type != "sessionRuntime" {
		return nil, unexpectedPayload("sessionRuntime", payload.Type)
	}
	if payload.RuntimeID == nil {
		return nil, nil
	}
	return &SessionRuntime{RuntimeID: *payload.RuntimeID, SupportsSteer: payload.SupportsSteer}, nil
}

func (c *Client) Prompt(ctx context.Context, sessionID, runtimeID, prompt string) error {
	payload, err := c.Request(ctx, sessionID, runtimeID, map[string]any{
		"type": "prompt", "prompt": prompt,
	})
	if err != nil {
		return err
	}
	if payload.Type != "ack" {
		return unexpectedPayload("ack", payload.Type)
	}
	return nil
}

func unexpectedPayload(want, got string) error {
	return fmt.Errorf("unexpected client-daemon response payload %q; want %q", got, want)
}
