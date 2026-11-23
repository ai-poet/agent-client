package operations

import (
	"embed"
	"sync"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

//go:embed skills
var builtinSkillsFS embed.FS

// BuiltinSkillOwner is the package prefix for application-embedded skills
// injected into CaptureResources without modifying the plugins/ submodule.
const BuiltinSkillOwner = "builtin"

var builtinPackages = sync.OnceValues(func() ([]Package, error) {
	skills := discoverSkills(builtinSkillsFS)
	if len(skills) == 0 {
		return nil, nil
	}
	digest, err := packageDigest(builtinSkillsFS)
	if err != nil {
		return nil, err
	}
	return []Package{{
		Reference: protocol.PluginReference{Name: BuiltinSkillOwner, Digest: digest},
		Skills:    skills,
		tree:      builtinSkillsFS,
		builtin:   true,
	}}, nil
})

// BuiltinSkillKeys returns the package:skill keys of all embedded built-in skills.
func BuiltinSkillKeys() ([]string, error) {
	pkgs, err := builtinPackages()
	if err != nil {
		return nil, err
	}
	var keys []string
	for _, p := range pkgs {
		for _, s := range p.Skills {
			keys = append(keys, p.Reference.Name+":"+s.Name)
		}
	}
	return keys, nil
}
