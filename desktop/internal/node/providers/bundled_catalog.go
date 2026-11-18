package providers

import (
	"sync"

	"github.com/ai-poet/agent-client/desktop/internal/appresources"
)

var bundledCatalog = sync.OnceValues(func() (*CatalogDownload, error) {
	raw, err := appresources.Read("providers/model-catalog.json")
	if err != nil {
		return nil, err
	}
	return ParseCatalog(raw)
})

// BundledCatalog provides the build's validated offline metadata snapshot.
// Its returned entries and bodies are shared and must not be modified.
func BundledCatalog() (*CatalogDownload, error) {
	return bundledCatalog()
}
