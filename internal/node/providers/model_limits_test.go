package providers

import (
	"encoding/json"
	"testing"
)

func TestCatalogModelLimits(t *testing.T) {
	catalog, err := ParseCatalog([]byte(`{
		"vendor-a":{"models":{"shared":{"id":"shared","limit":{"context":4096}},"placeholder":{"id":"placeholder","limit":{"context":4096}}}},
		"vendor-b":{"models":{"shared":{"id":"shared","limit":{"context":200000,"output":32768}}}},
		"vendor-c":{"models":{"shared":{"id":"shared","limit":{"context":64000,"output":8192}}}}
	}`))
	if err != nil {
		t.Fatal(err)
	}
	index := indexCatalogLimits(catalog.Entries)
	for _, row := range []struct {
		supplier, id string
		want         ModelLimits
	}{
		{"", "shared", ModelLimits{Context: 200000, Output: 32768}},
		{"vendor-a", "shared", ModelLimits{Context: 200000, Output: 32768}},
		{"vendor-c", "shared", ModelLimits{Context: 64000, Output: 8192}},
		{"", "vendor-c/shared", ModelLimits{Context: 64000, Output: 8192}},
		{"vendor-a", "placeholder", ModelLimits{}},
		{"vendor-a", "missing", ModelLimits{}},
	} {
		if limits := index.lookup(row.supplier, row.id); limits != row.want {
			t.Errorf("%s/%s = %+v, want %+v", row.supplier, row.id, limits, row.want)
		}
	}
	// In the public catalog an omitted limit stays omitted.
	index = indexCatalogLimits([]CatalogEntry{{Provider: "vendor", ID: "new", Body: json.RawMessage(`{"id":"new"}`)}})
	if got := index.lookup("vendor", "new"); got != (ModelLimits{}) {
		t.Fatalf("absent limits = %+v", got)
	}
}
