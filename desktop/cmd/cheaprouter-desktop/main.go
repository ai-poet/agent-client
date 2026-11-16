package main

import (
	"log"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/ai-poet/agent-client/desktop/internal/shell"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

func main() {
	model := shell.NewModel(daemon.NewSupervisor())
	defer func() {
		if err := model.Close(); err != nil {
			log.Printf("CheapRouter desktop shutdown: %v", err)
		}
	}()

	mygo.App.WhenReady(func() {
		mygo.NewWindow(mygo.WindowOptions{
			Title:     "CheapRouter",
			Width:     720,
			Height:    480,
			MinWidth:  520,
			MinHeight: 360,
			Content:   ui.View(model.View),
		})
	})
	model.Connect()
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
