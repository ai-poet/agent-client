package shell

import "github.com/ai-poet/cheaprouter-harness/internal/desktop/transfers"

func (a *App) SetInstallations(queue *transfers.Queue) { a.installs = queue }
