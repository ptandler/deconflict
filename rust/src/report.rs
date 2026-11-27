use console::{measure_text_width, style, Term};
use deconflict_core::{
    analysis::{consider, FileInfo, GroupAnalysis},
    scan::Group,
};
use std::collections::BTreeSet;

const FIELD_WIDTH: usize = 18;
const VALUE_WIDTH: usize = 43;
const MIN_WIDE_VALUE_WIDTH: usize = 18;
const META_VALUE_LIMIT: usize = 70;

pub(crate) struct GroupReport {
    pub(crate) text: String,
}

pub(crate) fn group_report(
    group: &Group,
    analysis: &GroupAnalysis,
    position: Option<(usize, usize)>,
    status: Option<&str>,
) -> GroupReport {
    group_report_with_width(group, analysis, position, status, terminal_width())
}

pub(crate) fn group_report_with_width(
    group: &Group,
    analysis: &GroupAnalysis,
    position: Option<(usize, usize)>,
    status: Option<&str>,
    width: usize,
) -> GroupReport {
    let title = group.base.file_name().unwrap_or_default().to_string_lossy();
    let heading = position.map_or_else(
        || format!("Group ready — {}", one_line(&title)),
        |(index, total)| format!("Group {index}/{total} — {}", one_line(&title)),
    );
    let mut lines = vec![String::new(), style(heading).bold().to_string()];
    if let Some(status) = status {
        lines.push(style(format!("status: {status}")).dim().to_string());
    }
    lines.extend([
        if analysis.all_equal {
            "all files identical in content".to_string()
        } else {
            "files differ in content".to_string()
        },
        "full paths:".into(),
    ]);
    if let Some(base) = &analysis.base {
        lines.push(format!("  {}", one_line(&base.path.display().to_string())));
    }
    for copy in &analysis.copies {
        lines.push(format!("  {}", one_line(&copy.path.display().to_string())));
    }

    if let Some(base) = &analysis.base {
        if let Some(table) = metadata_wide_table(analysis, width) {
            lines.push(table);
        } else {
            for copy_index in 0..analysis.copies.len() {
                lines.push(String::new());
                lines.push(metadata_diff_table(analysis, base, copy_index));
            }
        }
    }

    let consideration = consider(analysis, analysis.kind == "text");
    if !consideration.notes.is_empty() {
        lines.push(style("considered:").bold().to_string());
        lines.extend(consideration.notes.iter().map(|note| {
            let note = format!("  · {}", one_line(note));
            if note.contains("recommend keep ") {
                style(note).green().bold().to_string()
            } else {
                note
            }
        }));
    }
    GroupReport {
        text: lines.join("\n"),
    }
}

fn metadata_diff_table(analysis: &GroupAnalysis, base: &FileInfo, copy_index: usize) -> String {
    let copy = &analysis.copies[copy_index];
    let base_fields = analysis.meta_fields.first();
    let copy_fields = analysis.meta_fields.get(copy_index + 1);
    let content_cells = vec![
        "content".into(),
        content_base_cell(analysis, copy_index),
        content_copy_cell(analysis, copy_index),
    ];
    let content_row = TableRow {
        styles: vec![
            CellStyle::Plain,
            content_style(&content_cells[1]),
            content_style(&content_cells[2]),
        ],
        cells: content_cells,
    };
    let size_cells = vec![
        "size".into(),
        size_cell(base.size, None),
        size_cell(copy.size, Some(base.size)),
    ];
    let (base_size_style, copy_size_style) = if base.size > copy.size {
        (CellStyle::BoldYellow, CellStyle::Plain)
    } else if copy.size > base.size {
        (CellStyle::Plain, CellStyle::BoldYellow)
    } else {
        (CellStyle::Dim, CellStyle::Dim)
    };
    let created_cells = vec![
        "created".into(),
        field(base_fields, "created"),
        field(copy_fields, "created"),
    ];
    let created_style = if created_cells[1] == created_cells[2] {
        CellStyle::Dim
    } else {
        CellStyle::Plain
    };
    let modified_cells = vec![
        "modified".into(),
        field(base_fields, "modified"),
        field(copy_fields, "modified"),
    ];
    let modified_styles = match base.modified.cmp(&copy.modified) {
        std::cmp::Ordering::Less => [CellStyle::Plain, CellStyle::BoldCyan],
        std::cmp::Ordering::Equal => [CellStyle::Dim, CellStyle::Dim],
        std::cmp::Ordering::Greater => [CellStyle::BoldCyan, CellStyle::Plain],
    };
    let attribute_rows = vec![
        TableRow {
            cells: size_cells,
            styles: vec![CellStyle::Plain, base_size_style, copy_size_style],
        },
        TableRow {
            cells: created_cells,
            styles: vec![CellStyle::Plain, created_style, created_style],
        },
        TableRow {
            cells: modified_cells,
            styles: vec![CellStyle::Plain, modified_styles[0], modified_styles[1]],
        },
    ];
    let metadata_keys: BTreeSet<_> = base_fields
        .into_iter()
        .flat_map(|fields| fields.keys())
        .chain(copy_fields.into_iter().flat_map(|fields| fields.keys()))
        .filter(|key| !matches!(key.as_str(), "size" | "created" | "modified"))
        .collect();
    let mut common_rows = Vec::new();
    let mut differing_rows = Vec::new();
    for key in metadata_keys {
        let cells = vec![
            key.clone(),
            field(base_fields, key),
            field(copy_fields, key),
        ];
        let style = if cells[1] == cells[2] {
            CellStyle::Dim
        } else {
            CellStyle::Plain
        };
        let row = TableRow {
            cells,
            styles: vec![style, style, style],
        };
        if style == CellStyle::Dim {
            common_rows.push(row);
        } else {
            differing_rows.push(row);
        }
    }
    let headers = vec![
        "field".to_string(),
        format!("#0 {}", filename_header(base)),
        format!("#{} {}", copy_index + 1, filename_header(copy)),
    ];
    let mut sections = vec![vec![content_row], attribute_rows];
    if !common_rows.is_empty() {
        sections.push(common_rows);
    }
    if !differing_rows.is_empty() {
        sections.push(differing_rows);
    }
    render_table(headers, sections, &[FIELD_WIDTH, VALUE_WIDTH, VALUE_WIDTH])
}

fn metadata_wide_table(analysis: &GroupAnalysis, width: usize) -> Option<String> {
    let files: Vec<&FileInfo> = analysis.base.iter().chain(analysis.copies.iter()).collect();
    if files.len() < 3 {
        return None;
    }
    let columns = files.len() + 1;
    let value_width = wide_value_width(columns, width)?;
    let caps: Vec<usize> = std::iter::once(FIELD_WIDTH)
        .chain(std::iter::repeat_n(value_width, files.len()))
        .collect();

    let mut headers = vec!["field".to_string()];
    for (index, file) in files.iter().enumerate() {
        let name = file.path.file_name().unwrap_or_default().to_string_lossy();
        headers.push(wrap_header(&format!("#{index} {name}"), value_width));
    }

    let mut content_cells = vec!["content".to_string(), "base".to_string()];
    let mut content_styles = vec![CellStyle::Plain, CellStyle::Bold];
    for copy_index in 0..analysis.copies.len() {
        let cell = content_copy_cell(analysis, copy_index);
        content_styles.push(content_style(&cell));
        content_cells.push(cell);
    }
    let content_row = TableRow {
        cells: content_cells,
        styles: content_styles,
    };

    let size_values: Vec<String> = files
        .iter()
        .map(|file| size_cell(file.size, None))
        .collect();
    let all_sizes_equal = files.iter().all(|file| file.size == files[0].size);
    let largest_size = files.iter().map(|file| file.size).max().unwrap_or(0);
    let size_styles: Vec<CellStyle> = std::iter::once(CellStyle::Plain)
        .chain(
            files
                .iter()
                .map(|file| match (all_sizes_equal, file.size == largest_size) {
                    (true, _) => CellStyle::Dim,
                    (false, true) => CellStyle::BoldYellow,
                    (false, false) => CellStyle::Plain,
                }),
        )
        .collect();
    let size_row = TableRow {
        cells: std::iter::once("size".to_string())
            .chain(size_values)
            .collect(),
        styles: size_styles,
    };

    let created_values: Vec<String> = (0..files.len())
        .map(|index| field(analysis.meta_fields.get(index), "created"))
        .collect();
    let all_created_equal = created_values
        .iter()
        .all(|value| value == &created_values[0]);
    let created_style = if all_created_equal {
        CellStyle::Dim
    } else {
        CellStyle::Plain
    };
    let created_row = TableRow {
        cells: std::iter::once("created".to_string())
            .chain(created_values)
            .collect(),
        styles: std::iter::once(CellStyle::Plain)
            .chain(std::iter::repeat_n(created_style, files.len()))
            .collect(),
    };

    let modified_values: Vec<String> = (0..files.len())
        .map(|index| field(analysis.meta_fields.get(index), "modified"))
        .collect();
    let newest = files.iter().map(|file| file.modified).max();
    let modified_styles: Vec<CellStyle> = std::iter::once(CellStyle::Plain)
        .chain(files.iter().zip(&modified_values).map(|(file, value)| {
            if modified_values.iter().all(|other| other == value) {
                CellStyle::Dim
            } else if Some(file.modified) == newest {
                CellStyle::BoldCyan
            } else {
                CellStyle::Plain
            }
        }))
        .collect();
    let modified_row = TableRow {
        cells: std::iter::once("modified".to_string())
            .chain(modified_values)
            .collect(),
        styles: modified_styles,
    };

    let metadata_keys: BTreeSet<_> = analysis
        .meta_fields
        .iter()
        .flat_map(|fields| fields.keys())
        .filter(|key| !matches!(key.as_str(), "size" | "created" | "modified"))
        .collect();
    let mut common_rows = Vec::new();
    let mut differing_rows = Vec::new();
    for key in metadata_keys {
        let values: Vec<String> = (0..files.len())
            .map(|index| field(analysis.meta_fields.get(index), key))
            .collect();
        let all_equal = values.iter().all(|value| value == &values[0]);
        let style = if all_equal {
            CellStyle::Dim
        } else {
            CellStyle::Plain
        };
        let row = TableRow {
            cells: std::iter::once(key.clone()).chain(values).collect(),
            styles: std::iter::once(CellStyle::Plain)
                .chain(std::iter::repeat_n(style, files.len()))
                .collect(),
        };
        if all_equal {
            common_rows.push(row);
        } else {
            differing_rows.push(row);
        }
    }

    let mut sections = vec![vec![content_row], vec![size_row, created_row, modified_row]];
    if !common_rows.is_empty() {
        sections.push(common_rows);
    }
    if !differing_rows.is_empty() {
        sections.push(differing_rows);
    }
    Some(render_table(headers, sections, &caps))
}

fn wide_value_width(columns: usize, width: usize) -> Option<usize> {
    let overhead = 3 * columns + 1;
    let available = width.checked_sub(FIELD_WIDTH + overhead)?;
    let per_column = available / (columns - 1);
    (per_column >= MIN_WIDE_VALUE_WIDTH).then_some(per_column.min(VALUE_WIDTH))
}

fn terminal_width() -> usize {
    let (_, columns) = Term::stdout().size_checked().unwrap_or((24, 120));
    if columns == 0 {
        120
    } else {
        columns as usize
    }
}

fn wrap_header(value: &str, width: usize) -> String {
    let mut lines = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if !current.is_empty() && measure_text_width(&format!("{current}{ch}")) > width {
            lines.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > 3 {
        let tail = lines[2..].join(" ");
        lines.truncate(2);
        lines.push(truncate_cell(&tail, width));
    }
    lines.join("\n")
}

fn filename_header(file: &FileInfo) -> String {
    let name = file.path.file_name().unwrap_or_default().to_string_lossy();
    truncate_name(&name, 40)
}

fn truncate_name(value: &str, limit: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= limit {
        return value.to_owned();
    }
    let tail = 26.min(limit.saturating_sub(2));
    let head = limit - tail - 1;
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellStyle {
    Plain,
    Dim,
    Bold,
    BoldCyan,
    BoldYellow,
    Green,
    Yellow,
}

struct TableRow {
    cells: Vec<String>,
    styles: Vec<CellStyle>,
}

fn content_style(value: &str) -> CellStyle {
    match value {
        "identical" | "=" => CellStyle::Green,
        "differs" | "≠" => CellStyle::Yellow,
        _ => CellStyle::Plain,
    }
}

fn field(fields: Option<&std::collections::BTreeMap<String, String>>, key: &str) -> String {
    fields
        .and_then(|fields| fields.get(key))
        .map(|value| truncate_meta(value))
        .unwrap_or_default()
}

fn content_base_cell(analysis: &GroupAnalysis, copy_index: usize) -> String {
    let Some(base) = &analysis.base else {
        return String::new();
    };
    if base.sha256 == analysis.copies[copy_index].sha256 {
        "identical".into()
    } else if matches!(analysis.kind.as_str(), "text" | "office") {
        String::new()
    } else {
        "differs".into()
    }
}

fn content_copy_cell(analysis: &GroupAnalysis, copy_index: usize) -> String {
    let Some(base) = &analysis.base else {
        return "≠".into();
    };
    let copy = &analysis.copies[copy_index];
    if base.sha256 == copy.sha256 {
        return "=".into();
    }
    if !matches!(analysis.kind.as_str(), "text" | "office") {
        return "≠".into();
    }
    brief_text_diff(&base.path, &copy.path).unwrap_or_else(|| "≠".into())
}

fn brief_text_diff(base: &std::path::Path, copy: &std::path::Path) -> Option<String> {
    let base = std::fs::read(base).ok()?;
    let copy = std::fs::read(copy).ok()?;
    let base = String::from_utf8_lossy(&base);
    let copy = String::from_utf8_lossy(&copy);
    let base_lines: BTreeSet<_> = base.lines().collect();
    copy.lines()
        .find(|line| !base_lines.contains(line))
        .map(truncate_meta)
}

fn size_cell(size: u64, other: Option<u64>) -> String {
    let mut value = format!("{} ({} bytes)", human_size(size), grouped_number(size));
    if let Some(other) = other {
        if size == other {
            value.push_str(" (=)");
        } else {
            let (arrow, delta) = if size > other {
                ("↑", size - other)
            } else {
                ("↓", other - size)
            };
            value.push_str(&format!(" ({arrow} {})", human_size(delta)));
        }
    }
    value
}

fn human_size(size: u64) -> String {
    if size < 1024 {
        return format!("{size} B");
    }
    let mut value = size as f64;
    for unit in ["KB", "MB", "GB"] {
        value /= 1024.0;
        if value < 1024.0 || unit == "GB" {
            return format!("{value:.1} {unit}");
        }
    }
    unreachable!()
}

fn grouped_number(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let first_chunk = match digits.len() % 3 {
        0 => 3,
        remainder => remainder,
    };
    grouped.push_str(&digits[..first_chunk]);
    for start in (first_chunk..digits.len()).step_by(3) {
        grouped.push(',');
        grouped.push_str(&digits[start..start + 3]);
    }
    grouped
}

fn truncate_meta(value: &str) -> String {
    let value = one_line(value);
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= META_VALUE_LIMIT {
        return value;
    }
    format!(
        "{}… (+{} more)",
        chars[..META_VALUE_LIMIT].iter().collect::<String>(),
        chars.len() - META_VALUE_LIMIT
    )
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ' ',
            ch if ch.is_control() => '�',
            ch => ch,
        })
        .collect()
}

fn truncate_cell(value: &str, width: usize) -> String {
    let value = one_line(value);
    if measure_text_width(&value) <= width {
        return value;
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    for ch in value.chars() {
        if measure_text_width(&format!("{result}{ch}…")) > width {
            break;
        }
        result.push(ch);
    }
    result.push('…');
    result
}

fn pad_cell(value: &str, width: usize) -> String {
    let value = truncate_cell(value, width);
    let spaces = width.saturating_sub(measure_text_width(&value));
    format!("{value}{}", " ".repeat(spaces))
}

fn render_table(headers: Vec<String>, sections: Vec<Vec<TableRow>>, caps: &[usize]) -> String {
    let mut widths: Vec<usize> = caps
        .iter()
        .enumerate()
        .map(|(index, cap)| if index == 0 { *cap } else { 0 })
        .collect();
    for (column, header) in headers.iter().enumerate() {
        widths[column] = widths[column].max(cell_width(header, caps[column]));
    }
    for row in sections.iter().flatten() {
        for (column, value) in row.cells.iter().enumerate() {
            widths[column] = widths[column].max(cell_width(value, caps[column]));
        }
    }
    let border = |left: char, middle: char, right: char| {
        let segments: Vec<String> = widths.iter().map(|width| "─".repeat(width + 2)).collect();
        format!("{left}{}{right}", segments.join(&middle.to_string()))
    };
    let mut lines = vec![border('┌', '┬', '┐')];
    lines.extend(render_row(
        &headers,
        &vec![CellStyle::Bold; headers.len()],
        &widths,
    ));
    lines.push(border('├', '┼', '┤'));
    let mut has_rows = false;
    for section in sections.into_iter().filter(|section| !section.is_empty()) {
        if has_rows {
            lines.push(border('├', '┼', '┤'));
        }
        for row in &section {
            lines.extend(render_row(&row.cells, &row.styles, &widths));
        }
        has_rows = true;
    }
    lines.push(border('└', '┴', '┘'));
    format!("{}\n{}", style("metadata diff").bold(), lines.join("\n"))
}

fn cell_width(value: &str, cap: usize) -> usize {
    value
        .lines()
        .map(|line| measure_text_width(line).min(cap))
        .max()
        .unwrap_or(0)
}

fn render_row(cells: &[String], styles: &[CellStyle], widths: &[usize]) -> Vec<String> {
    let split: Vec<Vec<&str>> = cells
        .iter()
        .map(|cell| cell.split('\n').collect())
        .collect();
    let height = split.iter().map(Vec::len).max().unwrap_or(1);
    (0..height)
        .map(|line| {
            let columns: Vec<String> = split
                .iter()
                .enumerate()
                .map(|(column, lines)| {
                    styled_cell(
                        lines.get(line).copied().unwrap_or(""),
                        styles[column],
                        widths[column],
                    )
                })
                .collect();
            format!("│ {} │", columns.join(" │ "))
        })
        .collect()
}

fn styled_cell(value: &str, cell_style: CellStyle, width: usize) -> String {
    let padded = pad_cell(value, width);
    match cell_style {
        CellStyle::Plain => padded,
        CellStyle::Dim => style(padded).dim().to_string(),
        CellStyle::Bold => style(padded).bold().to_string(),
        CellStyle::BoldCyan => style(padded).cyan().bold().to_string(),
        CellStyle::BoldYellow => style(padded).yellow().bold().to_string(),
        CellStyle::Green => style(padded).green().to_string(),
        CellStyle::Yellow => style(padded).yellow().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deconflict_core::analysis::FileInfo;
    use std::{collections::BTreeMap, path::PathBuf, time::SystemTime};

    fn sample_file(name: &str, size: u64, sha: &str, seconds: u64) -> FileInfo {
        FileInfo {
            path: PathBuf::from(format!("/scan/{name}")),
            size,
            modified: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
            sha256: sha.into(),
            is_text: true,
        }
    }

    #[test]
    fn report_shows_position_and_current_pending_work() {
        let file = FileInfo {
            path: PathBuf::from("/scan/Readme.md"),
            size: 4,
            modified: SystemTime::UNIX_EPOCH,
            sha256: String::new(),
            is_text: true,
        };
        let analysis = GroupAnalysis {
            base: Some(file),
            copies: Vec::new(),
            all_equal: true,
            all_text: true,
            kind: "text".into(),
            meta: Vec::new(),
            meta_diff: Vec::new(),
            meta_fields: Vec::new(),
        };
        let group = Group {
            base: PathBuf::from("/scan/Readme.md"),
            conflicts: Vec::new(),
        };

        let report = group_report(
            &group,
            &analysis,
            Some((2, 5)),
            Some("scan in progress; 3 group(s) analyzed so far"),
        );
        assert!(report.text.contains("Group 2/5 — Readme.md"));
        assert!(report
            .text
            .contains("status: scan in progress; 3 group(s) analyzed so far"));
        assert!(report.text.contains("all files identical in content"));
    }

    #[test]
    fn wide_group_uses_one_table_and_falls_back_when_narrow() {
        let files = [
            sample_file("notes.md", 10, "base", 100),
            sample_file("notes (conflicted copy one).md", 20, "one", 200),
            sample_file("notes (conflicted copy two).md", 30, "two", 300),
            sample_file("notes (conflicted copy three).md", 40, "three", 400),
        ];
        let meta_fields = files
            .iter()
            .map(|file| {
                BTreeMap::from([
                    ("created".to_string(), "2026-01-01".to_string()),
                    (
                        "modified".to_string(),
                        format!("2026-02-0{}", file.size / 10),
                    ),
                    ("artist".to_string(), "sample".to_string()),
                ])
            })
            .collect();
        let analysis = GroupAnalysis {
            base: Some(files[0].clone()),
            copies: files[1..].to_vec(),
            all_equal: false,
            all_text: true,
            kind: "text".into(),
            meta: Vec::new(),
            meta_diff: Vec::new(),
            meta_fields,
        };
        let group = Group {
            base: files[0].path.clone(),
            conflicts: Vec::new(),
        };

        let wide = group_report_with_width(&group, &analysis, None, None, 120);
        assert_eq!(wide.text.matches("metadata diff").count(), 1);
        assert!(wide.text.contains("#0 notes.md"));
        assert!(
            wide.text.contains("#3 notes (conflicted copy three).md")
                || wide.text.contains("#3 notes")
        );
        assert!(wide.text.contains("│ content"));
        let content_line = wide
            .text
            .lines()
            .find(|line| line.trim_start().starts_with("│ content"))
            .unwrap();
        assert_eq!(content_line.matches('│').count(), 6);

        let narrow = group_report_with_width(&group, &analysis, None, None, 60);
        assert_eq!(narrow.text.matches("metadata diff").count(), 3);
    }
}
