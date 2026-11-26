use console::measure_text_width;
use deconflict_core::{
    analysis::{consider, FileInfo, GroupAnalysis, Recommendation},
    scan::Group,
};
use std::collections::BTreeSet;

const FIELD_WIDTH: usize = 18;
const VALUE_WIDTH: usize = 42;
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
        format!("Group {index}/{total} — {}", one_line(&title)),
        if analysis.all_equal {
            "all files identical in content".to_string()
        } else {
            "files differ in content".to_string()
        },
        "full paths:".into(),
    ];
    if let Some(base) = &analysis.base {
        lines.push(format!(
            "  #0 {}",
            one_line(&base.path.display().to_string())
        ));
    }
    for (copy_index, copy) in analysis.copies.iter().enumerate() {
        lines.push(format!(
            "  #{} {}",
            copy_index + 1,
            one_line(&copy.path.display().to_string())
        ));
    }

    if let Some(base) = &analysis.base {
        for copy_index in 0..analysis.copies.len() {
            lines.push(String::new());
            lines.push(metadata_diff_table(analysis, base, copy_index));
        }
    }

    let consideration = consider(analysis, analysis.kind == "text");
    if !consideration.notes.is_empty() {
        lines.push("considered:".into());
        lines.extend(
            consideration
                .notes
                .iter()
                .map(|note| format!("  · {}", one_line(note))),
        );
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
    let mut rows = vec![
        [
            "content".into(),
            content_base_cell(analysis, copy_index),
            content_copy_cell(analysis, copy_index),
        ],
        [
            "size".into(),
            size_cell(base.size, None),
            size_cell(copy.size, Some(base.size)),
        ],
        [
            "created".into(),
            field(base_fields, "created"),
            field(copy_fields, "created"),
        ],
        [
            "modified".into(),
            field(base_fields, "modified"),
            field(copy_fields, "modified"),
        ],
    ];
    let metadata_keys: BTreeSet<_> = base_fields
        .into_iter()
        .flat_map(|fields| fields.keys())
        .chain(copy_fields.into_iter().flat_map(|fields| fields.keys()))
        .filter(|key| !matches!(key.as_str(), "size" | "created" | "modified"))
        .collect();
    for key in metadata_keys {
        rows.push([
            key.clone(),
            field(base_fields, key),
            field(copy_fields, key),
        ]);
    }
    let headers = [
        "field".to_string(),
        format!(
            "#0 {}",
            base.path.file_name().unwrap_or_default().to_string_lossy()
        ),
        format!(
            "#{} {}",
            copy_index + 1,
            copy.path.file_name().unwrap_or_default().to_string_lossy()
        ),
    ];
    render_table(headers, rows)
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

fn render_table(headers: [String; 3], rows: Vec<[String; 3]>) -> String {
    let mut widths = [FIELD_WIDTH, 0, 0];
    for (column, header) in headers.iter().enumerate() {
        let cap = if column == 0 {
            FIELD_WIDTH
        } else {
            VALUE_WIDTH
        };
        widths[column] = widths[column].max(measure_text_width(header).min(cap));
    }
    for row in &rows {
        for (column, value) in row.iter().enumerate() {
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
        render_row(&headers, widths),
        border('├', '┼', '┤'),
    ];
    lines.extend(rows.iter().map(|row| render_row(row, widths)));
    lines.push(border('└', '┴', '┘'));
    format!("metadata diff\n{}", lines.join("\n"))
}

fn render_row(cells: &[String; 3], widths: [usize; 3]) -> String {
    format!(
        "│ {} │ {} │ {} │",
        pad_cell(&cells[0], widths[0]),
        pad_cell(&cells[1], widths[1]),
        pad_cell(&cells[2], widths[2])
    )
}
