package providers

import (
	"strings"
	"sync"

	"github.com/ai-poet/cheaprouter-harness/internal/appresources"
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

// BundledCatalogProvider returns the namespace for an unambiguous known
// endpoint. This hint is optional when a downloaded catalog is usable.
func BundledCatalogProvider(endpoint string) string {
	catalog, err := BundledCatalog()
	if err != nil {
		return ""
	}
	return catalog.endpoints[strings.TrimRight(endpoint, "/")]
}
