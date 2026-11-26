use console::{measure_text_width, style};
use deconflict_core::{
    analysis::{consider, FileInfo, GroupAnalysis, Recommendation},
    scan::Group,
};
use std::collections::BTreeSet;

const FIELD_WIDTH: usize = 18;
const VALUE_WIDTH: usize = 43;
const META_VALUE_LIMIT: usize = 70;

pub(crate) struct GroupReport {
    pub(crate) text: String,
    pub(crate) recommendation: Option<Recommendation>,
}

pub(crate) fn group_report(
    group: &Group,
    analysis: &GroupAnalysis,
    index: usize,
    total: usize,
) -> GroupReport {
    let title = group.base.file_name().unwrap_or_default().to_string_lossy();
    let mut lines = vec![
        String::new(),
        format!(
            "{}",
            style(format!("Group {index}/{total} — {}", one_line(&title))).bold()
        ),
        if analysis.all_equal {
            "all files identical in content".to_string()
        } else {
            "files differ in content".to_string()
        },
        "full paths:".into(),
    ];
    if let Some(base) = &analysis.base {
        lines.push(format!("  {}", one_line(&base.path.display().to_string())));
    }
    for copy in &analysis.copies {
        lines.push(format!("  {}", one_line(&copy.path.display().to_string())));
    }

    if let Some(base) = &analysis.base {
        for copy_index in 0..analysis.copies.len() {
            lines.push(String::new());
            lines.push(metadata_diff_table(analysis, base, copy_index));
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
        recommendation: consideration.recommendation,
    }
}

fn metadata_diff_table(analysis: &GroupAnalysis, base: &FileInfo, copy_index: usize) -> String {
    let copy = &analysis.copies[copy_index];
    let base_fields = analysis.meta_fields.first();
    let copy_fields = analysis.meta_fields.get(copy_index + 1);
    let content_cells = [
        "content".into(),
        content_base_cell(analysis, copy_index),
        content_copy_cell(analysis, copy_index),
    ];
    let content_row = TableRow {
        styles: [
            CellStyle::Plain,
            content_style(&content_cells[1]),
            content_style(&content_cells[2]),
        ],
        cells: content_cells,
    };
    let size_cells = [
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
    let created_cells = [
        "created".into(),
        field(base_fields, "created"),
        field(copy_fields, "created"),
    ];
    let created_style = if created_cells[1] == created_cells[2] {
        CellStyle::Dim
    } else {
        CellStyle::Plain
    };
    let modified_cells = [
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
            styles: [CellStyle::Plain, base_size_style, copy_size_style],
        },
        TableRow {
            cells: created_cells,
            styles: [CellStyle::Plain, created_style, created_style],
        },
        TableRow {
            cells: modified_cells,
            styles: [CellStyle::Plain, modified_styles[0], modified_styles[1]],
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
        let cells = [
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
            styles: [style, style, style],
        };
        if style == CellStyle::Dim {
            common_rows.push(row);
        } else {
            differing_rows.push(row);
        }
    }
    let headers = [
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
    render_table(headers, sections)
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
    cells: [String; 3],
    styles: [CellStyle; 3],
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

fn render_table(headers: [String; 3], sections: Vec<Vec<TableRow>>) -> String {
    let mut widths = [FIELD_WIDTH, 0, 0];
    for (column, header) in headers.iter().enumerate() {
        let cap = if column == 0 {
            FIELD_WIDTH
        } else {
            VALUE_WIDTH
        };
        widths[column] = widths[column].max(measure_text_width(header).min(cap));
    }
    for row in sections.iter().flatten() {
        for (column, value) in row.cells.iter().enumerate() {
            let cap = if column == 0 {
                FIELD_WIDTH
            } else {
                VALUE_WIDTH
            };
            widths[column] = widths[column].max(measure_text_width(value).min(cap));
        }
    }
    let border = |left: char, middle: char, right: char| {
        format!(
            "{left}{}{middle}{}{middle}{}{right}",
            "─".repeat(widths[0] + 2),
            "─".repeat(widths[1] + 2),
            "─".repeat(widths[2] + 2),
        )
    };
    let mut lines = vec![
        border('┌', '┬', '┐'),
        render_row(&headers, [CellStyle::Bold; 3], widths),
        border('├', '┼', '┤'),
    ];
    let mut has_rows = false;
    for section in sections.into_iter().filter(|section| !section.is_empty()) {
        if has_rows {
            lines.push(border('├', '┼', '┤'));
        }
        lines.extend(
            section
                .iter()
                .map(|row| render_row(&row.cells, row.styles, widths)),
        );
        has_rows = true;
    }
    lines.push(border('└', '┴', '┘'));
    format!("{}\n{}", style("metadata diff").bold(), lines.join("\n"))
}

fn render_row(cells: &[String; 3], styles: [CellStyle; 3], widths: [usize; 3]) -> String {
    format!(
        "│ {} │ {} │ {} │",
        styled_cell(&cells[0], styles[0], widths[0]),
        styled_cell(&cells[1], styles[1], widths[1]),
        styled_cell(&cells[2], styles[2], widths[2])
    )
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
