---
name: excel
metadata:
  display-name.en: "Excel"
  display-name.zh-CN: "Excel"
  display-name.zh-TW: "Excel"
  display-name.ja: "Excel"
  display-name.ko: "Excel"
  display-name.fr: "Excel"
  display-name.de: "Excel"
  display-name.es: "Excel"
  display-name.pt-BR: "Excel"
  display-name.ru: "Excel"
  display-name: Excel
description: Create, analyze and edit Excel workbooks (.xlsx), formulas, formatting, charts, tables and validations.
---

# Excel

## Execution

Sailry bundles Python 3.12.13. `load_skill` prepares this plugin's hash-locked, wheel-only environment and returns its absolute `python` and `directory`. Run scripts with `"$SAILRY_PYTHON"` and absolute resource paths. Commands use the environment of the most recently loaded skill in this turn; reload the required skill before running if another environment was loaded later. Do not copy the returned interpreter path. The shared environment and plugin resources are read-only during commands; save task scripts and temporary files in `SAILRY_SESSION_DIR`, and write requested deliverables to the worktree. Use the existing command tool and its normal permissions; skill loading grants no command permission. If preparation or execution fails, report the failure and keep the draft. Do not install globally or modify Sailry's application/shared environments. For extra task dependencies, use the Commands Python skill's session-local environment. Inspect installed library APIs with `help()` or `inspect` when needed.

Native `read_office` provides structured inspection for DOCX, XLSX, PPTX and PDF. Author and edit with the script libraries. Scripts use the existing command tool's permissions and working directory. If command execution is disabled or denied, report that constraint without bypassing it.

For existing files, inspect the original and preserve unrelated content. For focused script edits, record the source hash, load the existing file, save to a sibling temporary file, reopen and validate it, recheck the source hash immediately before replacement, then use `os.replace`. Preserve the original and report a conflict if its hash changed. Do not rebuild an existing file from extracted plain text. A library may drop unsupported package parts: inspect those parts and use a focused ZIP/XML patch when necessary. Keep relationships, namespaces and content types consistent.

Before any save, create the destination directory with `Path(destination).parent.mkdir(parents=True, exist_ok=True)`. This applies to documents, images and temporary output files. Verify saved content and the requested changes. Sailry previews Office files directly in the side panel; this does not establish print-layout fidelity. When the task requires rendered layout checks, use an available renderer and inspect representative pages with the image viewing tool. Keep editable source files. Return deliverables as ordinary standalone Markdown links, for example `[Budget](output/budget.xlsx)`. Sailry provides cards and side-panel previews; do not fabricate output paths or JSON cards.

## Spreadsheet authoring

Use the prepared `openpyxl` library for existing workbooks and complete workbook styling/editing APIs; use `xlsxwriter` for new workbooks with charts and explicit formula result caches. Use typed numeric/date values, formulas for derived data, named sheets, formats, widths, freeze panes, filters, tables, charts, validations, conditional formatting and print settings as requested.

`openpyxl` writes formulas but does not calculate them. `data_only=True` reads previous cached values; it is not recalculation. Keep formulas when loading files for editing (`data_only=False`). For new XlsxWriter workbooks, calculate and verify expected formula results and pass the cache value to `write_formula`. Report unverified or unsupported calculations honestly; never replace formulas with static values just to hide an error.

```python
from openpyxl import Workbook, load_workbook
from openpyxl.styles import Font, PatternFill
from openpyxl.chart import BarChart, Reference
from openpyxl.worksheet.datavalidation import DataValidation
book = Workbook()
sheet = book.active
sheet.title = 'Budget'
for row in [('Item', 'Amount'), ('Design', 120), ('Build', 240), ('Total', '=SUM(B2:B3)')]:
    sheet.append(row)
for cell in sheet[1]:
    cell.font = Font(bold=True, color='FFFFFF')
    cell.fill = PatternFill('solid', fgColor='245A81')
sheet.freeze_panes = 'A2'
sheet.column_dimensions['A'].width = 24
sheet.column_dimensions['B'].width = 18
chart = BarChart()
chart.add_data(Reference(sheet, min_col=2, min_row=1, max_row=3), titles_from_data=True)
chart.set_categories(Reference(sheet, min_col=1, min_row=2, max_row=3))
sheet.add_chart(chart, 'D2')
validation = DataValidation(type='decimal', operator='greaterThanOrEqual', formula1=0)
sheet.add_data_validation(validation)
validation.add('B2:B3')
book.save('budget.xlsx')
assert load_workbook('budget.xlsx')['Budget']['B4'].value == '=SUM(B2:B3)'
```

Inspect original formulas, ranges, styles and chart relationships before editing. `keep_vba=True` can preserve VBA parts in suitable files; it does not execute macros or guarantee every unsupported Excel feature survives. Check the requested values and formulas, totals, chart data and unchanged sheets after saving. The side-panel grid does not verify recalculation or print pagination.

## Package resources

- `templates/create.py` is an editable starter script, not a restricted authoring API. Read it with `read_skill_resource`, copy it into the worktree, then adapt its full library calls to the user's document. Run it with the selected project interpreter and an output path. It creates missing parent directories and refuses to replace an existing file.
- `scripts/inspect_file.py` prints the saved file's content for verification. Run `"$SAILRY_PYTHON" "<skill-directory>/scripts/inspect_file.py" "<file>"` using the absolute directory returned by `load_skill`. Quote paths and keep package resources unchanged. Structural inspection alone does not prove layout fidelity.

The plugin contains instructions, scripts and templates, not an interpreter. `read_office` is supplied by Sailry's Files capability; `run_command` uses Commands on the execution Node. If a required capability is unavailable, explain what must be enabled. Installing or enabling this skill does not grant execution permission.
