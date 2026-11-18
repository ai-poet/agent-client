package shell

import "github.com/ai-poet/agent-client/desktop/internal/desktop/transfers"

func (a *App) SetInstallations(queue *transfers.Queue) { a.installs = queue }
