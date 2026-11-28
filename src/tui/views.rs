//! Ratatui rendering. No filesystem mutation or terminal I/O happens here.

use super::{
    app::{App, ScrollMeta, ScrollTarget, Tab},
    images,
    session::Session,
};
use crate::{
    progress::{final_progress_line, live_progress_line},
    report::{content_tone, human_size, metadata_diff_table, size_cell, CellTone},
};
use chrono::{DateTime, Local};
use deconflict_core::{
    actions::{actions_for_with_pending, commands_for, ActionDescriptor, ActionKind},
    analysis::{consider, content_finding, FileInfo, Recommendation},
    config::Config,
    tools::Tools as ExternalTools,
};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, Tabs, Wrap,
    },
    Frame,
};
use std::io::{BufRead, BufReader};

fn accent() -> Color {
    if std::env::var_os("NO_COLOR").is_some() {
        Color::Reset
    } else {
        Color::Cyan
    }
}

fn good() -> Color {
    if std::env::var_os("NO_COLOR").is_some() {
        Color::Reset
    } else {
        Color::Green
    }
}

fn warn() -> Color {
    if std::env::var_os("NO_COLOR").is_some() {
        Color::Reset
    } else {
        Color::Yellow
    }
}

fn tone_style(tone: CellTone) -> Style {
    match tone {
        CellTone::Plain => Style::default(),
        CellTone::Dim => Style::default().add_modifier(Modifier::DIM),
        CellTone::Bold => Style::default().add_modifier(Modifier::BOLD),
        CellTone::BoldCyan => Style::default().fg(accent()).add_modifier(Modifier::BOLD),
        CellTone::BoldYellow => Style::default().fg(warn()).add_modifier(Modifier::BOLD),
        CellTone::Green => Style::default().fg(good()),
        CellTone::Yellow => Style::default().fg(warn()),
    }
}

fn selected_style(tone: CellTone) -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        return Style::default().add_modifier(Modifier::BOLD);
    }
    Style::default()
        .fg(tone_style(tone).fg.unwrap_or(Color::White))
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD)
}

fn styled_cell(text: impl Into<String>, tone: CellTone, selected: bool) -> Cell<'static> {
    let style = if selected {
        selected_style(tone)
    } else {
        tone_style(tone)
    };
    Cell::from(Span::styled(text.into(), style))
}

fn ansi_line(input: &str) -> Line<'static> {
    fn apply(style: Style, code: &str) -> Style {
        match code {
            "" | "0" => Style::default(),
            "1" => style.add_modifier(Modifier::BOLD),
            "2" => style.add_modifier(Modifier::DIM),
            "22" => style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            "32" => style.fg(good()),
            "33" => style.fg(warn()),
            "36" => style.fg(accent()),
            "39" => style.fg(Color::Reset),
            _ => style,
        }
    }

    let mut spans = Vec::new();
    let mut style = Style::default();
    let mut rest = input;
    while let Some(start) = rest.find('\x1b') {
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), style));
        }
        let Some(after) = rest.get(start + 2..) else {
            break;
        };
        let Some(end) = after.find('m') else {
            break;
        };
        for code in after[..end].split(';') {
            style = apply(style, code);
        }
        rest = &after[end + 1..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), style));
    }
    Line::from(spans)
}

fn file_relations(
    analysis: &deconflict_core::analysis::GroupAnalysis,
    index: usize,
) -> Vec<(&'static str, String, CellTone)> {
    let all = files(analysis);
    let Some(file) = all.get(index).copied() else {
        return Vec::new();
    };
    let is_base = index == 0 && analysis.base.is_some();
    let base = analysis.base.as_ref();
    let content = if is_base {
        "base".to_string()
    } else if let Some(base) = base {
        if file.sha256 == base.sha256 {
            "identical".to_string()
        } else {
            "differs".to_string()
        }
    } else {
        "differs".to_string()
    };
    let size = if is_base {
        "base".to_string()
    } else if let Some(base) = base {
        if file.size == base.size {
            "same".to_string()
        } else if file.size > base.size {
            format!("↑ {}", human_size(file.size - base.size))
        } else {
            format!("↓ {}", human_size(base.size - file.size))
        }
    } else {
        human_size(file.size)
    };
    let modified_label = if is_base {
        "base".to_string()
    } else if let Some(base) = base {
        if file.modified == base.modified {
            "same".to_string()
        } else if file.modified > base.modified {
            "newer".to_string()
        } else {
            "older".to_string()
        }
    } else {
        "different".to_string()
    };
    let size_tone = if is_base {
        CellTone::Bold
    } else if size.starts_with('↑') || size.starts_with('↓') {
        CellTone::BoldYellow
    } else if size == "same" {
        CellTone::Dim
    } else {
        CellTone::Plain
    };
    let modified_tone = match modified_label.as_str() {
        "base" => CellTone::Bold,
        "same" => CellTone::Dim,
        "newer" | "older" => CellTone::BoldCyan,
        _ => CellTone::Plain,
    };
    vec![
        (
            "content",
            content.clone(),
            if is_base {
                CellTone::Bold
            } else {
                content_tone(&content)
            },
        ),
        ("size", size, size_tone),
        ("modified", modified_label, modified_tone),
    ]
}

fn selected_findings(
    analysis: &deconflict_core::analysis::GroupAnalysis,
    index: usize,
) -> Vec<String> {
    let has_base = analysis.base.is_some();
    let which = if !has_base {
        Recommendation::Copy(index)
    } else if index == 0 {
        Recommendation::Base
    } else {
        Recommendation::Copy(index - 1)
    };
    let all = files(analysis);
    let mut findings = Vec::new();
    if analysis.winner_hint() == Some(which.clone()) {
        findings.push("newest".into());
    }
    if let (Some(base), Some(file)) = (analysis.base.as_ref(), all.get(index).copied()) {
        if file.size > base.size && all.iter().all(|other| other.size <= file.size) {
            findings.push("largest".into());
        }
    }
    if let Some(finding) = content_finding(analysis, &which) {
        findings.push(finding);
    }
    let consideration = consider(analysis, analysis.kind == "text");
    if consideration.recommendation.as_ref() == Some(&which) {
        findings.push("recommended".into());
    } else if consideration.recommendation.is_none() {
        findings.push("no clear recommendation".into());
    }
    findings
}

pub(super) fn draw(
    frame: &mut Frame,
    app: &mut App,
    session: &Session,
    config: &Config,
    tools: &ExternalTools,
) {
    let area = frame.area();
    let root = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!(
            "deconflict — resolve conflicted copies — conflicts: {}",
            session.groups.len()
        ))
        .style(Style::default().add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center),
        root[0],
    );

    app.main_area = root[1];
    let left_width = app.main_area.width.saturating_mul(app.split_percent) / 100;
    let columns = Layout::horizontal([
        Constraint::Length(left_width),
        Constraint::Length(1),
        Constraint::Min(12),
    ])
    .split(app.main_area);
    app.separator_x = columns[1].x;
    app.left_area = columns[0];
    app.right_area = columns[2];
    draw_groups(frame, app, session, columns[0]);
    frame.render_widget(
        Paragraph::new("│").style(Style::default().fg(accent())),
        columns[1],
    );
    draw_group_detail(frame, app, session, config, tools, columns[2]);
    draw_status(frame, app, session, root[2]);
}

fn marker(app: &App, session: &Session, index: usize) -> &'static str {
    let Some(entry) = session.groups.get(index) else {
        return " ";
    };
    if entry.skipped {
        "▸"
    } else if entry.decision.is_some() {
        "✓"
    } else if app.selected_group == index {
        ">"
    } else {
        " "
    }
}

fn wrapped_height(lines: &[Line<'_>], width: u16) -> usize {
    let width = width.max(1) as usize;
    lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width)
            }
        })
        .sum()
}

fn scroll_offset(app: &App, target: ScrollTarget) -> usize {
    match target {
        ScrollTarget::Groups => app.group_scroll,
        ScrollTarget::Detail => app.detail_offset(),
        ScrollTarget::FileDetail => app.file_detail_scroll,
    }
}

fn draw_scrollbar(
    frame: &mut Frame,
    app: &mut App,
    target: ScrollTarget,
    outer: Rect,
    content: usize,
    viewport: usize,
) {
    app.clamp_scroll(target);
    let track = (content > viewport && outer.width > 0 && outer.height > 0).then(|| Rect {
        x: outer.right() - 1,
        y: outer.y,
        width: 1,
        height: outer.height,
    });
    let offset = scroll_offset(app, target);
    app.set_scroll_meta(
        target,
        ScrollMeta {
            content,
            viewport,
            track,
        },
    );
    if let Some(track) = track {
        let mut state = ScrollbarState::new(content)
            .position(offset)
            .viewport_content_length(viewport);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            track,
            &mut state,
        );
    }
}

fn render_scrolled_lines(
    frame: &mut Frame,
    app: &mut App,
    target: ScrollTarget,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    wrap: bool,
) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let outer = block.inner(area);
    frame.render_widget(block, area);
    let viewport = outer.height as usize;
    let full_content = wrapped_height(&lines, outer.width);
    let content = if full_content > viewport {
        wrapped_height(&lines, outer.width.saturating_sub(1))
    } else {
        full_content
    };
    let visible = content > viewport;
    let content_area = if visible {
        Rect {
            width: outer.width.saturating_sub(1),
            ..outer
        }
    } else {
        outer
    };
    draw_scrollbar(frame, app, target, outer, content, viewport);
    let offset = scroll_offset(app, target).min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(lines).scroll((offset, 0));
    let paragraph = if wrap {
        paragraph.wrap(Wrap { trim: false })
    } else {
        paragraph
    };
    frame.render_widget(paragraph, content_area);
}

fn draw_groups(frame: &mut Frame, app: &mut App, session: &Session, area: Rect) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .title(" conflicts ");
    let outer = block.inner(area);
    frame.render_widget(block, area);
    let content = session.groups.len().max(1);
    let viewport = outer.height as usize;
    let visible = content > viewport;
    let inner = if visible {
        Rect {
            width: outer.width.saturating_sub(1),
            ..outer
        }
    } else {
        outer
    };
    draw_scrollbar(frame, app, ScrollTarget::Groups, outer, content, viewport);

    let mut rows = Vec::new();
    let mut rects = Vec::new();
    if session.groups.is_empty() {
        rows.push(Line::from(if session.scan_complete() {
            "no conflicts found"
        } else {
            "scanning for conflicts…"
        }));
    } else {
        let offset = app.group_scroll;
        for (local, index) in (offset..content).take(inner.height as usize).enumerate() {
            let entry = &session.groups[index];
            let selected = app.selected_group == index;
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(accent())
                    .add_modifier(Modifier::BOLD)
            } else if entry.is_decided() {
                Style::default().fg(good())
            } else {
                Style::default()
            };
            let state = if entry.analysis.is_none() {
                "…"
            } else if entry.skipped {
                "skipped"
            } else {
                entry.pattern.as_str()
            };
            let file_count = entry.group.conflicts.len() + 1;
            let label = if file_count > 2 {
                format!("{} ({}x)", entry.label, file_count)
            } else {
                entry.label.clone()
            };
            rows.push(Line::from(vec![
                Span::styled(marker(app, session, index), style),
                Span::raw(" "),
                Span::styled(
                    truncate(&label, inner.width.saturating_sub(14) as usize),
                    style,
                ),
                Span::styled(format!("  {state}"), style),
            ]));
            rects.push((
                Rect {
                    x: inner.x,
                    y: inner.y + local as u16,
                    width: inner.width,
                    height: 1,
                },
                index,
            ));
        }
    }
    app.group_rows = rects;
    frame.render_widget(Paragraph::new(rows), inner);
}

fn draw_group_detail(
    frame: &mut Frame,
    app: &mut App,
    session: &Session,
    config: &Config,
    tools: &ExternalTools,
    area: Rect,
) {
    let entry = app.selected_entry(session);
    let title = entry.map_or_else(
        || " no conflict selected ".to_string(),
        |entry| {
            format!(
                " {} ({}/{}) ",
                entry.label,
                app.selected_group + 1,
                session.groups.len()
            )
        },
    );
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(action_height(app, session, config, tools, area.width)),
    ])
    .split(area);
    app.ensure_visible_tab(session);
    let visible = app.visible_tabs(session);
    let label_width = visible
        .iter()
        .map(|tab| tab.title().chars().count())
        .max()
        .unwrap_or(0);
    let tabs = Tabs::new(
        visible
            .iter()
            .map(|tab| Line::from(format!(" {:width$} ", tab.title(), width = label_width)))
            .collect::<Vec<_>>(),
    )
    .select(visible.iter().position(|tab| *tab == app.tab).unwrap_or(0))
    .divider(" ")
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(accent())
            .add_modifier(Modifier::BOLD | Modifier::REVERSED),
    )
    .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(tabs, vertical[0]);
    app.tab_areas = tab_rects(vertical[0], &visible, label_width);
    app.image_area = None;

    match app.tab {
        Tab::Meta => draw_meta(frame, app, session, vertical[1]),
        Tab::Files => draw_files(frame, app, session, vertical[1]),
        Tab::Errors => draw_errors(frame, app, session, vertical[1]),
        Tab::Help => draw_help(frame, app, vertical[1]),
        Tab::Images => {
            let block = Block::default().borders(Borders::ALL).title(" images ");
            let inner = block.inner(vertical[1]);
            frame.render_widget(block, vertical[1]);
            app.image_area = Some(inner);
            let text = if images::kitty_supported() {
                "kitty inline images are drawn in this pane"
            } else {
                "inline images require a kitty terminal; use (v) to open the file externally"
            };
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
        }
        Tab::Diff | Tab::Log => {
            let text = if app.tab == Tab::Log {
                app.log.join("\n")
            } else if app.show_config {
                config_help(app, session, config, tools)
            } else {
                app.diff_text
                    .clone()
                    .unwrap_or_else(|| "press (d)iff for a text diff".into())
            };
            let lines = text
                .lines()
                .map(|line| Line::from(line.to_string()))
                .collect::<Vec<_>>();
            let title = format!(" {} ", app.tab.title());
            render_scrolled_lines(
                frame,
                app,
                ScrollTarget::Detail,
                vertical[1],
                &title,
                lines,
                true,
            );
        }
    }
    draw_actions(frame, app, session, config, tools, vertical[2]);
}

fn tab_rects(area: Rect, tabs: &[Tab], label_width: usize) -> Vec<(Rect, Tab)> {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    let mut x = inner.x;
    tabs.iter()
        .map(|tab| {
            let width = (label_width + 2) as u16;
            let available = inner.width.saturating_sub(x.saturating_sub(inner.x));
            let rect = Rect {
                x,
                y: inner.y,
                width: width.min(available),
                height: 1,
            };
            x = x.saturating_add(width.saturating_add(1));
            (rect, *tab)
        })
        .collect()
}

fn draw_meta(frame: &mut Frame, app: &mut App, session: &Session, area: Rect) {
    let Some(entry) = app.selected_entry(session) else {
        render_scrolled_lines(
            frame,
            app,
            ScrollTarget::Detail,
            area,
            " metadata ",
            vec![Line::from("no conflict selected")],
            false,
        );
        return;
    };
    let Some(stamped) = entry
        .analysis
        .as_ref()
        .and_then(|result| result.as_ref().ok())
    else {
        let text = match &entry.analysis {
            Some(Err(error)) => format!("analysis failed: {error}"),
            _ => "analysis in progress…".to_string(),
        };
        render_scrolled_lines(
            frame,
            app,
            ScrollTarget::Detail,
            area,
            " metadata ",
            vec![Line::from(text)],
            false,
        );
        return;
    };
    let analysis = &stamped.analysis;
    let mut lines = Vec::new();
    if let Some(recommendation) = App::recommendation(entry) {
        lines.push(Line::from(Span::styled(
            format!("recommend: {recommendation}"),
            Style::default().fg(good()).add_modifier(Modifier::BOLD),
        )));
    }
    if let Some(base) = &analysis.base {
        let copy_index = app.selected_file.saturating_sub(1);
        let table = metadata_diff_table(analysis, base, copy_index);
        lines.extend(table.lines().map(ansi_line));
        let note = deconflict_core::analysis::consider(analysis, analysis.kind == "text");
        for note in note.notes {
            lines.push(Line::from(format!("  · {note}")));
        }
    } else {
        lines.push(Line::from("no base file; metadata comparison unavailable"));
        for (index, file) in analysis.copies.iter().enumerate() {
            lines.push(Line::from(format!(
                "  #{index} {} — {}",
                file.path.display(),
                human_size(file.size)
            )));
        }
    }
    render_scrolled_lines(
        frame,
        app,
        ScrollTarget::Detail,
        area,
        " metadata ",
        lines,
        false,
    );
}

fn files(analysis: &deconflict_core::analysis::GroupAnalysis) -> Vec<&FileInfo> {
    analysis.base.iter().chain(analysis.copies.iter()).collect()
}

fn mtime(info: &FileInfo) -> String {
    DateTime::<Local>::from(info.modified)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn draw_files(frame: &mut Frame, app: &mut App, session: &Session, area: Rect) {
    let detail_height = area.height.saturating_div(3).clamp(4, 10);
    let panes = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(detail_height.min(area.height.saturating_sub(3))),
    ])
    .split(area);
    let block = Block::default().borders(Borders::ALL).title(" files ");
    let outer = block.inner(panes[0]);
    frame.render_widget(block, panes[0]);
    app.file_detail_area = panes[1];

    let Some(entry) = app.selected_entry(session) else {
        render_scrolled_lines(
            frame,
            app,
            ScrollTarget::Detail,
            panes[0],
            " files ",
            vec![Line::from("no conflict selected")],
            false,
        );
        app.file_rows.clear();
        return;
    };
    let Some(stamped) = entry
        .analysis
        .as_ref()
        .and_then(|result| result.as_ref().ok())
    else {
        render_scrolled_lines(
            frame,
            app,
            ScrollTarget::Detail,
            panes[0],
            " files ",
            vec![Line::from("analysis in progress…")],
            false,
        );
        app.file_rows.clear();
        return;
    };
    let analysis = &stamped.analysis;
    let all = files(analysis);
    let base_size = all.first().map(|file| file.size);
    let widths = [
        Constraint::Length(3),
        Constraint::Min(14),
        Constraint::Length(26),
        Constraint::Length(17),
        Constraint::Length(10),
    ];
    let viewport = outer.height.saturating_sub(1) as usize;
    let content = all.len();
    let visible = content > viewport;
    let table_area = if visible {
        Rect {
            width: outer.width.saturating_sub(1),
            ..outer
        }
    } else {
        outer
    };
    draw_scrollbar(frame, app, ScrollTarget::Detail, outer, content, viewport);
    let offset = app.detail_offset();
    let rows = all
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .map(|(index, file)| {
            let name = file.path.file_name().unwrap_or_default().to_string_lossy();
            let metadata = if index == 0 {
                "—".to_string()
            } else {
                match analysis.meta.get(index - 1) {
                    Some(Some(true)) => "same".to_string(),
                    Some(Some(false)) => "diff".to_string(),
                    _ => "—".to_string(),
                }
            };
            let metadata_tone = match metadata.as_str() {
                "same" => CellTone::Dim,
                "diff" => CellTone::Yellow,
                _ => CellTone::Plain,
            };
            let relations = file_relations(analysis, index);
            let selected = app.selected_file == index;
            Row::new(vec![
                styled_cell(format!("#{index}"), CellTone::Bold, selected),
                styled_cell(
                    truncate(&name, table_area.width.saturating_sub(56) as usize),
                    CellTone::Plain,
                    selected,
                ),
                styled_cell(
                    size_cell(file.size, base_size.filter(|_| index > 0)),
                    relations[1].2,
                    selected,
                ),
                styled_cell(mtime(file), relations[2].2, selected),
                styled_cell(metadata, metadata_tone, selected),
            ])
        });
    let table = Table::new(rows, widths)
        .header(
            Row::new(["#", "file", "size", "mtime", "meta"])
                .style(Style::default().fg(accent()).add_modifier(Modifier::BOLD)),
        )
        .block(Block::default());
    frame.render_widget(table, table_area);
    app.file_rows = all
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .enumerate()
        .map(|(local, (index, _))| {
            (
                Rect {
                    x: table_area.x,
                    y: table_area.y + 1 + local as u16,
                    width: table_area.width,
                    height: 1,
                },
                index,
            )
        })
        .collect();
    draw_file_detail(frame, app, analysis, panes[1]);
}

fn draw_file_detail(
    frame: &mut Frame,
    app: &mut App,
    analysis: &deconflict_core::analysis::GroupAnalysis,
    area: Rect,
) {
    let detail_width = Block::default().borders(Borders::ALL).inner(area).width;
    let Some(file) = files(analysis).get(app.selected_file).copied() else {
        render_scrolled_lines(
            frame,
            app,
            ScrollTarget::FileDetail,
            area,
            " file detail ",
            vec![Line::from("select a file")],
            false,
        );
        return;
    };
    let mut lines = Vec::new();
    if app.selected_file == 0 && analysis.base.is_some() {
        lines.push(Line::from(Span::styled(
            "select a copy to compare with #0 (base)",
            Style::default().fg(warn()).add_modifier(Modifier::BOLD),
        )));
    }
    let relations = file_relations(analysis, app.selected_file);
    let mut relation_spans = Vec::new();
    for (index, (label, value, tone)) in relations.iter().enumerate() {
        if index > 0 {
            relation_spans.push(Span::raw("  "));
        }
        relation_spans.push(Span::styled(
            format!("{label}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        relation_spans.push(Span::styled(value.clone(), tone_style(*tone)));
    }
    lines.push(Line::from(relation_spans));
    let findings = selected_findings(analysis, app.selected_file);
    if !findings.is_empty() {
        lines.push(Line::from(Span::styled(
            "findings:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for finding in findings {
            lines.push(Line::from(format!("  · {finding}")));
        }
    }
    let content_start = lines.len();
    if file.is_text {
        if let Ok(handle) = std::fs::File::open(&file.path) {
            for (index, line) in BufReader::new(handle).lines().take(200).enumerate() {
                match line {
                    Ok(line) => lines.push(Line::from(format!("{:>3} │ {line}", index + 1))),
                    Err(error) => lines.push(Line::from(format!("read error: {error}"))),
                }
            }
        }
        if lines.len() == content_start {
            lines.push(Line::from("(empty file)"));
        }
    } else if let Some(fields) = analysis.meta_fields.get(app.selected_file) {
        for (name, value) in fields {
            lines.push(Line::from(format!(
                "{name}: {}",
                truncate(value, detail_width as usize)
            )));
        }
    } else {
        lines.push(Line::from(format!(
            "{} ({})",
            file.path.display(),
            human_size(file.size)
        )));
    }
    render_scrolled_lines(
        frame,
        app,
        ScrollTarget::FileDetail,
        area,
        " file detail ",
        lines,
        false,
    );
}

fn file_type_actions(actions: Vec<ActionDescriptor>) -> Vec<ActionDescriptor> {
    actions
        .into_iter()
        .filter(|action| {
            !matches!(
                action.kind,
                ActionKind::Resolve(_)
                    | ActionKind::Skip
                    | ActionKind::Quit
                    | ActionKind::ForceQuit
                    | ActionKind::Config
                    | ActionKind::Metadata
            )
        })
        .collect()
}

fn action_mode_label(action: &ActionDescriptor) -> String {
    action
        .mode
        .map(|mode| format!("{mode:?}"))
        .unwrap_or_else(|| "built-in".to_string())
}

fn config_help(app: &App, session: &Session, config: &Config, tools: &ExternalTools) -> String {
    let analysis = app
        .selected_entry(session)
        .and_then(|entry| entry.analysis.as_ref())
        .and_then(|result| result.as_ref().ok());
    let file_type = analysis
        .map(|stamped| stamped.analysis.kind.clone())
        .unwrap_or_else(|| "unknown".into());
    let mut lines = vec![format!("file type: {file_type}")];
    let Some(stamped) = analysis else {
        lines.push("analysis in progress…".into());
        return lines.join("\n");
    };
    let pending = !session.scan_complete() || !session.analysis_complete();
    let selected = app
        .selected_view_path(session)
        .map(std::path::PathBuf::from);
    let actions = file_type_actions(actions_for_with_pending(
        &stamped.analysis,
        config,
        tools,
        pending,
    ));
    for action in actions {
        lines.push(format!(
            "({}) {} [{}]",
            action.key,
            action.label,
            action_mode_label(&action)
        ));
        if matches!(action.kind, ActionKind::Tool { .. }) {
            match commands_for(&stamped.analysis, &action, tools, selected.as_deref()) {
                Ok(commands) if commands.is_empty() => lines.push("  no files available".into()),
                Ok(commands) => {
                    for command in commands {
                        lines.push(format!(
                            "  {} {}",
                            command.program.display(),
                            command
                                .args
                                .iter()
                                .map(|arg| arg.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join(" ")
                        ));
                    }
                }
                Err(error) => lines.push(format!("  {error}")),
            }
        }
    }
    lines.join("\n")
}

fn action_width(action: &deconflict_core::actions::ActionDescriptor) -> usize {
    action.label.len() + action.key.len() + 5
}

fn action_rows(
    actions: &[deconflict_core::actions::ActionDescriptor],
    width: usize,
) -> Vec<Vec<usize>> {
    let mut rows = Vec::new();
    let mut current = Vec::new();
    let mut used = 0usize;
    for (index, action) in actions.iter().enumerate() {
        let button_width = action_width(action);
        if !current.is_empty() && used.saturating_add(button_width) > width.max(12) {
            rows.push(std::mem::take(&mut current));
            used = 0;
        }
        used += button_width;
        current.push(index);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

fn action_height(
    app: &App,
    session: &Session,
    config: &Config,
    tools: &ExternalTools,
    width: u16,
) -> u16 {
    let actions = app.actions(session, config, tools);
    if actions.is_empty() {
        return 3;
    }
    (action_rows(&actions, width.saturating_sub(4) as usize).len() + 3) as u16
}

fn draw_actions(
    frame: &mut Frame,
    app: &mut App,
    session: &Session,
    config: &Config,
    tools: &ExternalTools,
    area: Rect,
) {
    let actions = app.actions(session, config, tools);
    app.action_areas.clear();
    let block = Block::default().borders(Borders::ALL).title(" actions ");
    frame.render_widget(block, area);
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if actions.is_empty() {
        frame.render_widget(
            Paragraph::new(if session.scan_complete() {
                "no actions available"
            } else {
                "scanning…"
            }),
            inner,
        );
        return;
    }
    let mut rows = action_rows(&actions, inner.width as usize);
    let available = usize::from(inner.height.saturating_sub(1));
    if rows.len() > available {
        rows.truncate(available);
    }
    for (row_index, indices) in rows.iter().enumerate() {
        let y = inner.y + row_index as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let total: usize = indices
            .iter()
            .map(|index| action_width(&actions[*index]))
            .sum();
        let mut x = inner.x;
        for &action_index in indices {
            let action = &actions[action_index];
            let remaining = usize::from(inner.width.saturating_sub(x.saturating_sub(inner.x)));
            let width = action_width(action)
                .checked_mul(inner.width as usize)
                .and_then(|value| value.checked_div(total))
                .map_or(1, |value| value.max(1).min(remaining)) as u16;
            let rect = Rect {
                x,
                y,
                width,
                height: 1,
            };
            let color = match &action.kind {
                ActionKind::Resolve(_) => good(),
                ActionKind::ForceQuit => Color::Red,
                ActionKind::Quit | ActionKind::Skip => warn(),
                _ => accent(),
            };
            frame.render_widget(
                Paragraph::new(truncate(
                    &format!("({}) {}", action.key, action.label),
                    width as usize,
                ))
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center),
                rect,
            );
            app.action_areas.push((rect, action_index));
            x = x.saturating_add(width);
        }
    }
    let hint_y = inner.y + rows.len() as u16;
    if hint_y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new("press (h) to list keyboard shortcuts")
                .style(Style::default().fg(warn()))
                .alignment(Alignment::Center),
            Rect {
                x: inner.x,
                y: hint_y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn draw_errors(frame: &mut Frame, app: &mut App, session: &Session, area: Rect) {
    let status = session.status();
    let mut lines = Vec::new();
    for (path, error) in &status.error_messages {
        lines.push(Line::from(format!("{}: {}", path.display(), error)));
    }
    for entry in &session.groups {
        if let Some(Err(error)) = &entry.analysis {
            lines.push(Line::from(format!(
                "{}: analysis failed: {error}",
                entry.label
            )));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from("no errors"));
    }
    render_scrolled_lines(
        frame,
        app,
        ScrollTarget::Detail,
        area,
        "errors",
        lines,
        true,
    );
}

fn draw_help(frame: &mut Frame, app: &mut App, area: Rect) {
    let lines = [
        "Enter  accept recommendation",
        "0..9   keep base / keep copy N",
        "b      keep both",
        "s      skip this conflict",
        "d/v/e/x/t/g  context-dependent diff, view or edit actions (shown when available)",
        "m      show metadata",
        "?      show tools and config",
        "q      quit immediately (cancels scan and discards recorded choices)",
        "Q      quit when scan is done (only while work is pending)",
        "h      show this help",
        "Tab / Shift-Tab / Ctrl-T  cycle tabs",
        "Left / Right  previous / next tab",
        "Up / Down  choose conflict; in files tab choose file",
        "Ctrl-Up / Ctrl-Down  choose conflict regardless of tab",
        "PageUp / PageDown, Ctrl-PageUp / Ctrl-PageDown  page conflicts",
        "Shift-Up / Shift-Down  scroll the right pane one line",
        "Shift-PageUp / Shift-PageDown  scroll the right pane one page",
        "Home / End  first / last conflict",
        "Esc / Ctrl-C  interrupt scan",
        "mouse  click tab, action, conflict or file; drag divider to resize",
        "mouse wheel / drag scrollbar  scroll conflicts or right pane",
    ]
    .iter()
    .map(|line| Line::from(Span::raw(*line)))
    .collect::<Vec<_>>();
    render_scrolled_lines(
        frame,
        app,
        ScrollTarget::Detail,
        area,
        "keyboard shortcuts",
        lines,
        true,
    );
}

fn draw_status(frame: &mut Frame, _app: &App, session: &Session, area: Rect) {
    let status = session.status();
    let decided = session
        .groups
        .iter()
        .filter(|entry| entry.is_decided())
        .count();
    let columns = area.width.saturating_sub(2) as usize;
    let (analyses_done, analyses_queued) = session.group_analysis_counts();
    let pending = !session.scan_complete() || !session.analysis_complete();
    let progress = if pending {
        live_progress_line(
            status.files,
            status.directories,
            analyses_done,
            analyses_queued,
            session.estimated_file_count(),
            columns,
            session.elapsed(),
            "conflicts",
        )
    } else {
        final_progress_line(
            status.files,
            status.directories,
            analyses_done,
            analyses_queued,
            session.estimated_file_count(),
            columns,
            session.elapsed(),
            "conflicts",
        )
    };
    let phase = if !session.scan_complete() {
        "scanning"
    } else if !session.analysis_complete() {
        "analyzing"
    } else {
        "ready"
    };
    let tally = format!(
        "{decided} decided; {} error(s){}",
        status.errors,
        if session.options_dry_run() {
            " [dry-run]"
        } else {
            ""
        }
    );
    let first = format!("{progress} | {phase} | {tally}");
    let second = if pending {
        status
            .current_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "scanning…".into())
    } else {
        session
            .timing_summary()
            .unwrap_or_else(|| "scan complete".into())
    };
    let lines = vec![
        Line::from(truncate(&first, columns)),
        Line::from(Span::styled(
            truncate(&second, columns),
            Style::default().fg(accent()),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn truncate(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= width {
        return value.to_string();
    }
    if width == 1 {
        return "…".into();
    }
    let mut result: String = chars[..width - 1].iter().collect();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_uses_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn wrapped_height_counts_wrapped_lines() {
        let lines = vec![Line::from("short"), Line::from("1234567890")];
        assert_eq!(wrapped_height(&lines, 10), 2);
        assert_eq!(wrapped_height(&lines, 5), 3);
    }

    #[test]
    fn scrollbar_appears_only_for_overflowing_content() {
        use ratatui::{backend::TestBackend, Terminal};

        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = App::new();
        let short = (0..2)
            .map(|index| Line::from(format!("line {index}")))
            .collect::<Vec<_>>();
        terminal
            .draw(|frame| {
                render_scrolled_lines(
                    frame,
                    &mut app,
                    ScrollTarget::Detail,
                    frame.area(),
                    " test ",
                    short.clone(),
                    false,
                )
            })
            .expect("short draw");
        assert!(app.detail_scroll_meta.track.is_none());

        let long = (0..20)
            .map(|index| Line::from(format!("line {index}")))
            .collect::<Vec<_>>();
        terminal
            .draw(|frame| {
                render_scrolled_lines(
                    frame,
                    &mut app,
                    ScrollTarget::Detail,
                    frame.area(),
                    " test ",
                    long.clone(),
                    false,
                )
            })
            .expect("long draw");
        assert!(app.detail_scroll_meta.track.is_some());
        let symbols = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(symbols.contains('█') || symbols.contains('║'), "{symbols}");
    }

    #[test]
    fn tab_areas_cover_each_visible_tab() {
        let area = Rect {
            x: 2,
            y: 3,
            width: 100,
            height: 3,
        };
        let areas = tab_rects(area, &Tab::ALL, 9);
        assert_eq!(areas.len(), Tab::ALL.len());
        assert_eq!(areas[0].1, Tab::Meta);
        assert_eq!(areas[0].0.y, 4);
        assert!(areas.iter().all(|(rect, _)| rect.width == 11));
        assert!(areas.windows(2).all(|pair| pair[0].0.x + 12 == pair[1].0.x));
    }

    #[test]
    fn file_type_actions_keep_only_file_actions() {
        use deconflict_core::{actions::ToolOperation, resolve::Decision};

        let action = |key: &str, kind: ActionKind| ActionDescriptor {
            key: key.into(),
            label: key.into(),
            kind,
            mode: None,
        };
        let actions = vec![
            action("Enter", ActionKind::Resolve(Decision::KeepBase)),
            action("s", ActionKind::Skip),
            action("q", ActionKind::Quit),
            action("Q", ActionKind::ForceQuit),
            action("?", ActionKind::Config),
            action("d", ActionKind::TextDiff),
            action("i", ActionKind::ViewImages),
            action("m", ActionKind::Metadata),
            action(
                "v",
                ActionKind::Tool {
                    operation: ToolOperation::View,
                    tool_name: "viewer".into(),
                },
            ),
        ];
        let keys = file_type_actions(actions)
            .into_iter()
            .map(|action| action.key)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["d", "i", "v"]);
    }

    #[test]
    fn action_mode_label_drops_option_wrapper() {
        use deconflict_core::tools::ToolMode;

        let tool = ActionDescriptor {
            key: "e".into(),
            label: "edit".into(),
            kind: ActionKind::Tool {
                operation: deconflict_core::actions::ToolOperation::Edit,
                tool_name: "fresh".into(),
            },
            mode: Some(ToolMode::Tui),
        };
        let builtin = ActionDescriptor {
            key: "d".into(),
            label: "diff".into(),
            kind: ActionKind::TextDiff,
            mode: None,
        };
        assert_eq!(action_mode_label(&tool), "Tui");
        assert_eq!(action_mode_label(&builtin), "built-in");
    }

    #[test]
    fn selected_cell_has_readable_contrast() {
        let style = selected_style(CellTone::Dim);
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(!style.add_modifier.contains(Modifier::DIM));
        if std::env::var_os("NO_COLOR").is_some() {
            assert_eq!(style.bg, None);
        } else {
            assert_eq!(style.bg, Some(Color::Indexed(236)));
            assert_eq!(style.fg, Some(Color::White));
        }
    }

    #[test]
    fn ansi_line_preserves_table_colours() {
        let line = ansi_line("\x1b[33mdiffers\x1b[0m plain \x1b[36mnewer\x1b[0m");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, "differs");
        assert_eq!(line.spans[0].style.fg, Some(warn()));
        assert_eq!(line.spans[1].content, " plain ");
        assert_eq!(line.spans[1].style.fg, None);
        assert_eq!(line.spans[2].content, "newer");
        assert_eq!(line.spans[2].style.fg, Some(accent()));
    }

    #[test]
    fn file_relations_report_differences() {
        use deconflict_core::analysis::GroupAnalysis;
        use std::{path::PathBuf, time::Duration, time::SystemTime};
        let file =
            |name: &str, size: u64, sha: &str, seconds: u64| deconflict_core::analysis::FileInfo {
                path: PathBuf::from(name),
                size,
                modified: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
                sha256: sha.into(),
                is_text: true,
            };
        let analysis = GroupAnalysis {
            base: Some(file("base.txt", 10, "base", 100)),
            copies: vec![file("copy.txt", 20, "copy", 200)],
            all_equal: false,
            all_text: true,
            kind: "text".into(),
            meta: vec![None],
            meta_diff: vec![None],
            meta_fields: Vec::new(),
        };
        let relations = file_relations(&analysis, 1);
        assert_eq!(relations[0].1, "differs");
        assert_eq!(relations[0].2, CellTone::Yellow);
        assert_eq!(relations[1].1, "↑ 10 B");
        assert_eq!(relations[1].2, CellTone::BoldYellow);
        assert_eq!(relations[2].1, "newer");
        assert_eq!(relations[2].2, CellTone::BoldCyan);
    }

    #[test]
    fn selected_findings_include_analysis_flags() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base.txt");
        let copy = dir.path().join("copy.txt");
        std::fs::write(&base, "one\ntwo\n").unwrap();
        std::fs::write(&copy, "one\ntwo\nthree\n").unwrap();
        let mut analysis = deconflict_core::analysis::analyze_group(&base, &[copy]).unwrap();
        let newer = analysis.base.as_ref().unwrap().modified + std::time::Duration::from_secs(1);
        analysis.copies[0].modified = newer;
        let findings = selected_findings(&analysis, 1);
        assert!(findings.iter().any(|finding| finding == "newest"));
        assert!(findings.iter().any(|finding| finding == "largest"));
        assert!(findings
            .iter()
            .any(|finding| finding == "additions only (nothing removed)"));
        assert!(findings.iter().any(|finding| finding == "recommended"));
    }
}
