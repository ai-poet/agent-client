package store

import (
	"context"
	"database/sql"
	"fmt"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func storageNode(ctx context.Context, db *sql.Conn) (protocol.NodeID, error) {
	var raw []byte
	if err := db.QueryRowContext(ctx, "SELECT identity FROM node WHERE singleton=1").Scan(&raw); err != nil {
		return protocol.NodeID{}, err
	}
	if len(raw) != 32 {
		return protocol.NodeID{}, fmt.Errorf("invalid storage Node identity")
	}
	return protocol.NodeID(raw), nil
}
