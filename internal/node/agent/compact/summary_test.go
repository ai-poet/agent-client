package compact

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/tokens"
)

func TestSummaryCapFollowsWindow(t *testing.T) {
	for _, row := range []struct {
		name                     string
		context, modelOutput     uint64
		output, limit            uint64
		target, cut, absentLimit string
	}{
		{"4k window", 4096, 8000, 1024, 512, "under 384 tokens", "beyond 512 tokens", "4,000"},
		{"8k window", 8192, 8000, 2048, 1024, "under 768 tokens", "beyond 1,024 tokens", "4,000"},
		{"8k window with lower model output", 8192, 1024, 1024, 512, "under 384 tokens", "beyond 512 tokens", "4,000"},
		{"32k window is unchanged", 32000, 8000, 8000, 4000, "under 3,000 tokens", "beyond 4,000 tokens", ""},
		{"128k window is unchanged", 128000, 64000, 8000, 4000, "under 3,000 tokens", "beyond 4,000 tokens", ""},
		{"200k window is unchanged", 200000, 64000, 8000, 4000, "under 3,000 tokens", "beyond 4,000 tokens", ""},
		{"400k window", 400000, 64000, 16000, 8000, "under 6,000 tokens", "beyond 8,000 tokens", "4,000"},
		{"1M window", 1000000, 64000, 24000, 12000, "under 9,000 tokens", "beyond 12,000 tokens", "4,000"},
		{"2M window stays at the maximum", 2000000, 64000, 24000, 12000, "under 9,000 tokens", "beyond 12,000 tokens", "4,000"},
		{"model output bounds the cap", 1000000, 10000, 10000, 5000, "under 3,750 tokens", "beyond 5,000 tokens", "12,000"},
		{"small model output bounds the cap", 1000000, 4096, 4096, 2048, "under 1,536 tokens", "beyond 2,048 tokens", "12,000"},
	} {
		t.Run(row.name, func(t *testing.T) {
			output := min(Output(row.context), row.modelOutput)
			if output != row.output || Cap(row.context, output) != row.limit {
				t.Fatalf("output = %d, cap = %d, want %d and %d", output, Cap(row.context, output), row.output, row.limit)
			}
			prompt, err := Prepare(nil, row.context, output)
			if err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(prompt, row.target) || !strings.Contains(prompt, row.cut) || strings.Contains(prompt, "{{") || row.absentLimit != "" && strings.Contains(prompt, row.absentLimit+" tokens") {
				t.Fatalf("prompt limits: %s", prompt[strings.Index(prompt, "Keep the whole handoff"):strings.Index(prompt, "## Output")])
			}
		})
	}
	t.Run("bound", func(t *testing.T) {
		work := strings.Repeat("Preserve state and complete the task. ", 600)
		var concepts []string
		for range 400 {
			concepts = append(concepts, strings.Repeat("Disposable reference ", 10))
		}
		document, _ := json.Marshal(map[string]any{"user_intent": []string{"Fix parser"}, "current_work": work, "technical_concepts": concepts})
		small, err := Parse(string(document), 4000)
		if err != nil {
			t.Fatal(err)
		}
		large, err := Parse(string(document), 12000)
		if err != nil {
			t.Fatal(err)
		}
		smallSize, largeSize := tokens.Text(small.Render()), tokens.Text(large.Render())
		if smallSize > 4000 || largeSize > 12000 || largeSize <= 4000 {
			t.Fatalf("rendered sizes = %d and %d", smallSize, largeSize)
		}
		if len(large.TechnicalConcepts) <= len(small.TechnicalConcepts) || large.CurrentWork == nil || *large.CurrentWork != work {
			t.Fatal("the larger cap kept no more of the handoff")
		}
	})
	t.Run("small cap bounds the rendered handoff", func(t *testing.T) {
		document, _ := json.Marshal(map[string]any{"user_intent": []string{"Fix parser"}, "current_work": strings.Repeat("Preserve state and complete the task. ", 600)})
		limit := Cap(4096, Output(4096))
		summary, err := Parse(string(document), limit)
		if err != nil {
			t.Fatal(err)
		}
		if size := tokens.Text(summary.Render()); size > limit || !strings.Contains(summary.Render(), "Fix parser") {
			t.Fatalf("rendered handoff is %d tokens under cap %d", size, limit)
		}
		if _, err := Parse(`{"user_intent":["Complete the remaining task"]}`, 1); err == nil {
			t.Fatal("accepted a handoff whose essential fields exceed the cap")
		}
	})
}

// Bound must arrive where dropping one entry at a time arrives.
func TestBoundMatchesStepwiseTrim(t *testing.T) {
	build := func() *Summary {
		s := &Summary{UserIntent: []string{"Fix parser"}, PendingTasks: []string{"Add regression tests"}}
		for i := range 40 {
			s.TechnicalConcepts = append(s.TechnicalConcepts, strings.Repeat("concept ", i%7+1))
			s.ErrorsAndFixes = append(s.ErrorsAndFixes, strings.Repeat("fault fixed ", i%5+1))
			s.ProblemSolving = append(s.ProblemSolving, strings.Repeat("step taken ", i%3+1))
		}
		return s
	}
	full := tokens.Text(build().Render())
	for _, limit := range []uint64{full, full - 1, full * 3 / 4, full / 2, full / 3, full / 8, 60, 20} {
		stepwise, bisected := build(), build()
		for tokens.Text(stepwise.Render()) > limit && stepwise.trim() {
		}
		bisected.Bound(limit)
		if stepwise.Render() != bisected.Render() {
			t.Fatalf("limit %d: bisection kept %d/%d/%d entries, stepwise %d/%d/%d", limit,
				len(bisected.TechnicalConcepts), len(bisected.ErrorsAndFixes), len(bisected.ProblemSolving),
				len(stepwise.TechnicalConcepts), len(stepwise.ErrorsAndFixes), len(stepwise.ProblemSolving))
		}
	}
}

func TestParseStructuredAndLenientOutputs(t *testing.T) {
	t.Run("accepted formats", func(t *testing.T) {
		for _, tc := range []struct {
			name        string
			input       string
			wantIntent  string
			wantWork    string
			wantNext    string
			wantFile    string
			wantConcept string
		}{
			{
				name:       "plain structured json",
				input:      `{"user_intent":["Fix compaction parser"],"current_work":"Updated parser","next_step":"Run tests"}`,
				wantIntent: "Fix compaction parser",
				wantWork:   "Updated parser",
				wantNext:   "Run tests",
			},
			{
				name:       "fenced json lowercase",
				input:      "```json\n{\"user_intent\":[\"Handle fenced JSON\"]}\n```",
				wantIntent: "Handle fenced JSON",
			},
			{
				name:       "fenced JSON uppercase with surrounding prose",
				input:      "Here is the handoff summary:\n\n```JSON\n{\n  \"user_intent\": [\"Extract uppercase fence\"],\n  \"current_work\": \"In progress\"\n}\n```\n\nLet me know if you need more details.",
				wantIntent: "Extract uppercase fence",
				wantWork:   "In progress",
			},
			{
				name:       "fenced without language tag",
				input:      "Sure, here is the summary:\n```\n{\"user_intent\":[\"Unlabelled fence\"],\"next_step\":\"Verify\"}\n```",
				wantIntent: "Unlabelled fence",
				wantNext:   "Verify",
			},
			{
				name:       "tilde fenced json",
				input:      "~~~json\n{\"user_intent\":[\"Tilde fence\"]}\n~~~",
				wantIntent: "Tilde fence",
			},
			{
				name:       "xml summary tag wrapping json",
				input:      "<analysis>thinking...</analysis>\n<summary>\n{\"user_intent\":[\"XML wrapped handoff\"],\"current_work\":\"Ready\"}\n</summary>",
				wantIntent: "XML wrapped handoff",
				wantWork:   "Ready",
			},
			{
				name:       "prose before and after multiline json",
				input:      "下面是会话交接摘要：\n\n{\n  \"user_intent\": [\"修复上下文压缩\"],\n  \"current_work\": \"已完成解析增强\",\n  \"next_step\": \"运行单元测试\"\n}\n\n请基于以上信息继续。",
				wantIntent: "修复上下文压缩",
				wantWork:   "已完成解析增强",
				wantNext:   "运行单元测试",
			},
			{
				name:       "colon introduced single-line json with trailing note",
				input:      "Here is the structured handoff: {\"user_intent\":[\"Colon handoff\"],\"next_step\":\"Continue\"}\nHope that helps!",
				wantIntent: "Colon handoff",
				wantNext:   "Continue",
			},
			{
				name:        "lenient camelCase and alias fields with trailing comma",
				input:       "```json\n{\n  \"goal\": \"Align context limits\",\n  \"state\": \"Investigating providers\",\n  \"nextSteps\": [\"Patch cheaprouter.go\", \"Run go test\"],\n  \"keyFiles\": [{\"file\": \"providers/cheaprouter.go\", \"description\": \"Catalog normalization\"}],\n  \"essentialContext\": [\"min window 8192\",],\n}\n```",
				wantIntent:  "Align context limits",
				wantWork:    "Investigating providers",
				wantNext:    "Patch cheaprouter.go; Run go test",
				wantFile:    "providers/cheaprouter.go",
				wantConcept: "min window 8192",
			},
			{
				name:       "wrapped handoff object",
				input:      `{"handoff":{"userIntent":["Unwrap outer object"],"currentWork":"Done"}}`,
				wantIntent: "Unwrap outer object",
				wantWork:   "Done",
			},
			{
				name:       "multiline json with unescaped newline inside string",
				input:      "{\n  \"user_intent\": [\"Line one\nLine two\"],\n  \"current_work\": \"Active\"\n}",
				wantIntent: "Line one\nLine two",
				wantWork:   "Active",
			},
		} {
			t.Run(tc.name, func(t *testing.T) {
				summary, err := Parse(tc.input, 4000)
				if err != nil {
					t.Fatalf("Parse failed: %v", err)
				}
				if tc.wantIntent != "" && (len(summary.UserIntent) == 0 || summary.UserIntent[0] != tc.wantIntent) {
					t.Fatalf("UserIntent = %v, want %q", summary.UserIntent, tc.wantIntent)
				}
				if tc.wantWork != "" && (summary.CurrentWork == nil || *summary.CurrentWork != tc.wantWork) {
					t.Fatalf("CurrentWork = %v, want %q", summary.CurrentWork, tc.wantWork)
				}
				if tc.wantNext != "" && (summary.NextStep == nil || *summary.NextStep != tc.wantNext) {
					t.Fatalf("NextStep = %v, want %q", summary.NextStep, tc.wantNext)
				}
				if tc.wantFile != "" && (len(summary.Files) == 0 || summary.Files[0].Path != tc.wantFile) {
					t.Fatalf("Files = %+v, want path %q", summary.Files, tc.wantFile)
				}
				if tc.wantConcept != "" && (len(summary.TechnicalConcepts) == 0 || summary.TechnicalConcepts[0] != tc.wantConcept) {
					t.Fatalf("TechnicalConcepts = %v, want %q", summary.TechnicalConcepts, tc.wantConcept)
				}
			})
		}
	})

	t.Run("rejected non-json outputs", func(t *testing.T) {
		for _, tc := range []struct {
			name  string
			input string
		}{
			{"plain paragraph", "We worked on fixing the compaction parser and need to run tests next."},
			{"markdown sections only", "## Goal\n- Fix compaction\n\n## Next Steps\n- Run tests"},
			{"empty text", "   \n\t  "},
			{"inline prose example", `A quoted example {"user_intent":["Wrong"]} is not a handoff`},
			{"array document", "```json\n[\n{\"user_intent\":[\"Wrong\"]}\n]\n```"},
			{"unfinished analysis", "<analysis>\n```json\n{\"user_intent\":[\"Private draft\"]}\n```"},
			{"truncated json", "```json\n{\"user_intent\":[\"Keep\"],\"pending_tasks\":[\"Unfinished"},
		} {
			t.Run(tc.name, func(t *testing.T) {
				if _, err := Parse(tc.input, 4000); err == nil {
					t.Fatalf("Parse unexpectedly accepted %q", tc.name)
				}
			})
		}
	})
}
