//! Fork addition: the Files panel's previews of files that are not text.
//!
//! Read on the daemon host like every other workspace file, so a remote
//! workspace previews too. Pictures travel as their bytes; spreadsheets as
//! the first rows and columns of each sheet (calamine reads Excel and
//! OpenDocument, and CSV / TSV are parsed here); Word, PowerPoint and
//! OpenDocument text as their words. Everything is capped, so a preview of a
//! huge file costs a bounded read and a bounded message.

use std::fs;
use std::io::Read as _;
use std::path::Path;

use anyhow::Context as _;
use base64::Engine as _;
use calamine::Reader as _;
use quick_xml::events::Event;
use waku_protocol::workspace::{FilePreview, PreviewKind, PreviewSheet, preview_kind};

use super::resolve_workspace_path;

/// A picture larger than this is described, not sent.
const IMAGE_LIMIT: u64 = 30 * 1024 * 1024;
/// Spreadsheets and documents larger than this are not opened at all.
const PARSE_LIMIT: u64 = 100 * 1024 * 1024;
/// How much of a CSV is read for its first rows.
const DELIMITED_READ: u64 = 16 * 1024 * 1024;
const MAX_ROWS: usize = 1_000;
const MAX_COLUMNS: usize = 100;
const MAX_SHEETS: usize = 30;
const MAX_CELL_CHARS: usize = 500;
const DOCUMENT_CHARS: usize = 200_000;

pub(super) fn preview_file(root: &Path, relative: &Path) -> anyhow::Result<FilePreview> {
    let path = resolve_workspace_path(root, relative)?;
    let size = fs::metadata(&path)
        .with_context(|| format!("could not read {}", relative.display()))?
        .len();
    let extension = extension_of(relative);
    let kind = preview_kind(&relative.to_string_lossy());
    let preview = match kind {
        Some(PreviewKind::Image) => image(&path, &extension, size),
        Some(PreviewKind::Table) if size > PARSE_LIMIT && !is_delimited(&extension) => {
            Ok(too_large(size))
        }
        Some(PreviewKind::Table) if is_delimited(&extension) => delimited(&path, &extension, size),
        Some(PreviewKind::Table) => spreadsheet(&path, size),
        Some(PreviewKind::Document) if size > PARSE_LIMIT => Ok(too_large(size)),
        Some(PreviewKind::Document) => document(&path, &extension, size),
        Some(PreviewKind::Binary) | None => Ok(FilePreview::Unavailable { size, reason: None }),
    };
    // A file that claims a format and is not in it is described, not failed:
    // the person can still open it with something else.
    Ok(preview.unwrap_or_else(|error| FilePreview::Unavailable {
        size,
        reason: Some(format!("{error:#}")),
    }))
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_delimited(extension: &str) -> bool {
    matches!(extension, "csv" | "tsv")
}

fn too_large(size: u64) -> FilePreview {
    FilePreview::Unavailable {
        size,
        reason: Some("the file is too large to preview".to_owned()),
    }
}

fn image(path: &Path, extension: &str, size: u64) -> anyhow::Result<FilePreview> {
    if size > IMAGE_LIMIT {
        return Ok(too_large(size));
    }
    let format = match extension {
        "jpg" | "jpeg" => "jpeg",
        "tif" | "tiff" => "tiff",
        other => other,
    };
    let bytes = fs::read(path)?;
    Ok(FilePreview::Image {
        format: format.to_owned(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        size,
    })
}

fn cell(text: String) -> String {
    if text.chars().count() > MAX_CELL_CHARS {
        let mut cut: String = text.chars().take(MAX_CELL_CHARS).collect();
        cut.push('…');
        cut
    } else {
        text
    }
}

/// A cell as a person reads it. calamine prints a date as Excel's serial
/// number and a computed number with its binary-float tail; neither is what
/// the sheet shows.
fn cell_text(value: &calamine::Data) -> String {
    match value {
        calamine::Data::Float(number) => number_text(*number),
        calamine::Data::DateTime(date) if date.is_duration() => date
            .as_duration()
            .map(|duration| {
                let seconds = duration.num_seconds();
                format!(
                    "{}:{:02}:{:02}",
                    seconds / 3600,
                    (seconds % 3600) / 60,
                    seconds % 60
                )
            })
            .unwrap_or_else(|| number_text(date.as_f64())),
        calamine::Data::DateTime(date) => date
            .as_datetime()
            .map(|moment| {
                if moment.time() == chrono::NaiveTime::MIN {
                    moment.format("%Y-%m-%d").to_string()
                } else {
                    moment.format("%Y-%m-%d %H:%M:%S").to_string()
                }
            })
            .unwrap_or_else(|| number_text(date.as_f64())),
        other => other.to_string(),
    }
}

/// At most ten decimals, without trailing zeros: `0.1 + 0.2` reads `0.3`.
fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        return format!("{number:.0}");
    }
    let text = format!("{number:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

fn spreadsheet(path: &Path, size: u64) -> anyhow::Result<FilePreview> {
    let mut workbook = calamine::open_workbook_auto(path).context("not a readable spreadsheet")?;
    let mut sheets = Vec::new();
    for name in workbook.sheet_names().into_iter().take(MAX_SHEETS) {
        let Ok(range) = workbook.worksheet_range(&name) else {
            continue;
        };
        let (first_row, first_column) = range
            .start()
            .map(|(row, column)| (row as usize, column as usize))
            .unwrap_or_default();
        let rows = range
            .rows()
            .take(MAX_ROWS)
            .map(|row| {
                row.iter()
                    .take(MAX_COLUMNS)
                    .map(|value| cell(cell_text(value)))
                    .collect()
            })
            .collect();
        sheets.push(PreviewSheet {
            name,
            first_row,
            first_column,
            rows,
            total_rows: range.height(),
            total_columns: range.width(),
        });
    }
    Ok(FilePreview::Table { sheets, size })
}

fn delimited(path: &Path, extension: &str, size: u64) -> anyhow::Result<FilePreview> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(DELIMITED_READ)
        .read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let delimiter = if extension == "tsv" { '\t' } else { ',' };
    let (rows, total_rows, total_columns) = parse_delimited(text, delimiter, size > DELIMITED_READ);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(FilePreview::Table {
        sheets: vec![PreviewSheet {
            name,
            first_row: 0,
            first_column: 0,
            rows,
            total_rows,
            total_columns,
        }],
        size,
    })
}

/// RFC 4180 fields: quoted fields may hold the delimiter, newlines and
/// doubled quotes. Keeps the first [`MAX_ROWS`] rows and counts the rest.
/// When the text was cut short, its last, likely partial, row is dropped.
fn parse_delimited(text: &str, delimiter: char, cut_short: bool) -> (Vec<Vec<String>>, usize, usize) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut total_rows = 0;
    let mut total_columns = 0;
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut finish_row = |row: &mut Vec<String>, rows: &mut Vec<Vec<String>>| {
        total_rows += 1;
        total_columns = total_columns.max(row.len());
        let row = std::mem::take(row);
        if rows.len() < MAX_ROWS {
            rows.push(row.into_iter().take(MAX_COLUMNS).map(cell).collect());
        }
    };
    while let Some(character) = chars.next() {
        if quoted {
            match character {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                other => field.push(other),
            }
            continue;
        }
        match character {
            '"' if field.is_empty() => quoted = true,
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                finish_row(&mut row, &mut rows);
            }
            other if other == delimiter => row.push(std::mem::take(&mut field)),
            other => field.push(other),
        }
    }
    if !cut_short && (!field.is_empty() || !row.is_empty()) {
        row.push(field);
        finish_row(&mut row, &mut rows);
    }
    (rows, total_rows, total_columns)
}

fn document(path: &Path, extension: &str, size: u64) -> anyhow::Result<FilePreview> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).context("not a readable document")?;
    let parts: Vec<String> = match extension {
        "docx" => vec!["word/document.xml".to_owned()],
        "pptx" => {
            let mut slides: Vec<(usize, String)> = archive
                .file_names()
                .filter_map(|name| {
                    let number = name
                        .strip_prefix("ppt/slides/slide")?
                        .strip_suffix(".xml")?
                        .parse()
                        .ok()?;
                    Some((number, name.to_owned()))
                })
                .collect();
            slides.sort();
            slides.into_iter().map(|(_, name)| name).collect()
        }
        _ => vec!["content.xml".to_owned()],
    };
    let mut text = String::new();
    for (index, part) in parts.iter().enumerate() {
        let mut xml = String::new();
        archive
            .by_name(part)
            .with_context(|| format!("the document has no {part}"))?
            .read_to_string(&mut xml)?;
        if extension == "pptx" {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(&format!("— {} —\n", index + 1));
        }
        text.push_str(&xml_text(&xml));
        if text.chars().count() > DOCUMENT_CHARS {
            break;
        }
    }
    let truncated = text.chars().count() > DOCUMENT_CHARS;
    if truncated {
        text = text.chars().take(DOCUMENT_CHARS).collect();
    }
    Ok(FilePreview::Document {
        text: text.trim_end().to_owned(),
        truncated,
        size,
    })
}

/// The words of an Office or OpenDocument XML part: text runs, a line break
/// after each paragraph or heading, tabs and explicit breaks kept.
fn xml_text(xml: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(run)) => {
                if let Ok(decoded) = run.decode() {
                    text.push_str(&decoded);
                }
            }
            Ok(Event::CData(run)) => {
                if let Ok(decoded) = run.decode() {
                    text.push_str(&decoded);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if let Ok(Some(character)) = reference.resolve_char_ref() {
                    text.push(character);
                } else if let Ok(name) = reference.decode() {
                    text.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::End(end)) => {
                if matches!(end.local_name().as_ref(), b"p" | b"h") {
                    text.push('\n');
                }
            }
            Ok(Event::Empty(element)) => match element.local_name().as_ref() {
                b"tab" => text.push('\t'),
                b"br" | b"line-break" | b"cr" => text.push('\n'),
                b"p" | b"h" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::*;

    fn workspace() -> PathBuf {
        let root = std::env::temp_dir().join(format!("waku-preview-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn csv_fields_keep_quotes_commas_and_newlines() {
        let (rows, total, columns) = parse_delimited(
            "name,note\r\n\"Smith, J\",\"said \"\"hi\"\"\nthen left\"\nplain,\n",
            ',',
            false,
        );
        assert_eq!(total, 3);
        assert_eq!(columns, 2);
        assert_eq!(rows[1], vec!["Smith, J", "said \"hi\"\nthen left"]);
        assert_eq!(rows[2], vec!["plain", ""]);

        let (rows, total, _) = parse_delimited("a\tb\nc\td", '\t', false);
        assert_eq!(total, 2);
        assert_eq!(rows[1], vec!["c", "d"]);
    }

    #[test]
    fn a_long_csv_keeps_its_first_rows_and_counts_the_rest() {
        let text: String = (0..1_500).map(|row| format!("{row},x\n")).collect();
        let (rows, total, _) = parse_delimited(&text, ',', false);
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(total, 1_500);
        // Read only partly: the last line may be cut, so it is dropped.
        let (rows, total, _) = parse_delimited("a,b\nc,d\ne,f", ',', true);
        assert_eq!(total, 2);
        assert_eq!(rows.last().unwrap(), &vec!["c".to_owned(), "d".to_owned()]);
    }

    #[test]
    fn cells_read_as_the_sheet_shows_them() {
        assert_eq!(number_text(0.1 + 0.2), "0.3");
        assert_eq!(number_text(1200.5), "1200.5");
        assert_eq!(number_text(42.0), "42");
        assert_eq!(number_text(-0.0000000000001), "0");
        // 45000 is 2023-03-15 in Excel's 1900 date system.
        let date = calamine::Data::DateTime(calamine::ExcelDateTime::new(
            45000.0,
            calamine::ExcelDateTimeType::DateTime,
            false,
        ));
        assert_eq!(cell_text(&date), "2023-03-15");
        let moment = calamine::Data::DateTime(calamine::ExcelDateTime::new(
            45000.5,
            calamine::ExcelDateTimeType::DateTime,
            false,
        ));
        assert_eq!(cell_text(&moment), "2023-03-15 12:00:00");
        assert_eq!(cell_text(&calamine::Data::String("x".to_owned())), "x");
        assert_eq!(cell_text(&calamine::Data::Empty), "");
    }

    #[test]
    fn document_xml_reads_as_paragraphs() {
        let xml = r#"<w:document xmlns:w="w"><w:body>
            <w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve">A &amp; B</w:t></w:r></w:p>
            <w:p><w:r><w:t>Second</w:t><w:br/><w:t>line &#169;</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = xml_text(xml);
        assert!(text.contains("Hello\tA & B\n"), "{text:?}");
        assert!(text.contains("Second\nline ©\n"), "{text:?}");
    }

    #[test]
    fn pictures_travel_as_their_bytes_and_unknown_files_are_described() {
        let root = workspace();
        fs::write(root.join("dot.png"), [0x89, b'P', b'N', b'G']).unwrap();
        let preview = preview_file(&root, Path::new("dot.png")).unwrap();
        assert_eq!(
            preview,
            FilePreview::Image {
                format: "png".to_owned(),
                data: "iVBORw==".to_owned(),
                size: 4,
            }
        );
        fs::write(root.join("song.mp3"), [0u8; 10]).unwrap();
        assert_eq!(
            preview_file(&root, Path::new("song.mp3")).unwrap(),
            FilePreview::Unavailable { size: 10, reason: None }
        );
        // A spreadsheet that is not one says why instead of failing.
        fs::write(root.join("fake.xlsx"), b"not a zip").unwrap();
        assert!(matches!(
            preview_file(&root, Path::new("fake.xlsx")).unwrap(),
            FilePreview::Unavailable { reason: Some(_), .. }
        ));
        assert!(preview_file(&root, Path::new("../escape.png")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    fn write_archive(path: &Path, parts: &[(&str, &str)]) {
        let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, content) in parts {
            archive
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            archive.write_all(content.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }

    /// A real workbook's structure, its used range starting at B2.
    #[test]
    fn a_spreadsheet_shows_its_sheets_from_where_their_data_starts() {
        let root = workspace();
        write_archive(
            &root.join("budget.xlsx"),
            &[
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
                ),
                (
                    "xl/workbook.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="B2" t="inlineStr"><is><t>Item</t></is></c><c r="C2" t="inlineStr"><is><t>Cost</t></is></c></row><row r="3"><c r="B3" t="inlineStr"><is><t>Rent</t></is></c><c r="C3"><v>1200.5</v></c></row></sheetData></worksheet>"#,
                ),
            ],
        );
        let FilePreview::Table { sheets, .. } = preview_file(&root, Path::new("budget.xlsx")).unwrap()
        else {
            panic!("a table preview");
        };
        assert_eq!(sheets.len(), 1);
        let sheet = &sheets[0];
        assert_eq!(sheet.name, "Budget");
        assert_eq!((sheet.first_row, sheet.first_column), (1, 1));
        assert_eq!((sheet.total_rows, sheet.total_columns), (2, 2));
        assert_eq!(sheet.rows[0], vec!["Item", "Cost"]);
        assert_eq!(sheet.rows[1], vec!["Rent", "1200.5"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_word_document_reads_through_its_archive() {
        let root = workspace();
        let mut archive = zip::ZipWriter::new(fs::File::create(root.join("note.docx")).unwrap());
        archive
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        archive
            .write_all(br#"<w:document xmlns:w="w"><w:p><w:r><w:t>Quarterly plan</w:t></w:r></w:p></w:document>"#)
            .unwrap();
        archive.finish().unwrap();
        let FilePreview::Document { text, truncated, .. } =
            preview_file(&root, Path::new("note.docx")).unwrap()
        else {
            panic!("a document preview");
        };
        assert_eq!(text, "Quarterly plan");
        assert!(!truncated);
        fs::remove_dir_all(root).unwrap();
    }
}
