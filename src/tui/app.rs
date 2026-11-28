//! TUI state, input handling and action dispatch. Rendering lives in `views`.

use super::session::{GroupEntry, Session};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use deconflict_core::{
    actions::{
        actions_for_with_pending, commands_for, quit_actions, text_diff, view_paths,
        ActionDescriptor, ToolCommand,
    },
    analysis::{recommend, recommendation_label},
    config::Config,
    tools::{ToolMode, Tools as ExternalTools},
};
use ratatui::layout::{Position, Rect};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    Meta,
    Files,
    Diff,
    Images,
    #[default]
    Log,
    Errors,
    Help,
}

impl Tab {
    #[cfg(test)]
    pub(super) const ALL: [Self; 7] = [
        Self::Meta,
        Self::Files,
        Self::Diff,
        Self::Images,
        Self::Log,
        Self::Errors,
        Self::Help,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::Files => "files",
            Self::Diff => "diff",
            Self::Images => "images",
            Self::Log => "log",
            Self::Errors => "errors",
            Self::Help => "help",
        }
    }

    #[cfg(test)]
    fn next(self) -> Self {
        let index = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    #[cfg(test)]
    fn previous(self) -> Self {
        let index = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Default)]
pub(super) struct App {
    pub(super) split_percent: u16,
    pub(super) selected_group: usize,
    pub(super) selected_file: usize,
    pub(super) tab: Tab,
    pub(super) log: Vec<String>,
    pub(super) should_exit: bool,
    pub(super) dragging_split: bool,
    pub(super) show_config: bool,
    pub(super) group_rows: Vec<Rect>,
    pub(super) file_rows: Vec<Rect>,
    pub(super) tab_areas: Vec<(Rect, Tab)>,
    pub(super) action_areas: Vec<(Rect, usize)>,
    pub(super) main_area: Rect,
    pub(super) separator_x: u16,
    pending_tool: Option<(std::path::PathBuf, Vec<ToolCommand>)>,
    pub(super) diff_text: Option<String>,
    pub(super) last_image_request: Option<(std::path::PathBuf, std::path::PathBuf)>,
    pub(super) image_area: Option<Rect>,
}

impl App {
    pub(super) fn new() -> Self {
        Self {
            split_percent: 38,
            selected_group: 0,
            selected_file: 0,
            tab: Tab::Log,
            log: vec![
                "deconflict TUI — q quit, Q quit when done while scanning, h shortcuts".into(),
            ],
            should_exit: false,
            dragging_split: false,
            show_config: false,
            group_rows: Vec::new(),
            file_rows: Vec::new(),
            tab_areas: Vec::new(),
            action_areas: Vec::new(),
            main_area: Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            separator_x: 0,
            pending_tool: None,
            diff_text: None,
            last_image_request: None,
            image_area: None,
        }
    }

    pub(super) fn sync(&mut self, session: &Session) {
        let count = session.groups.len();
        if count == 0 {
            self.selected_group = 0;
        } else {
            self.selected_group = self.selected_group.min(count - 1);
        }
        let files = self
            .selected_entry(session)
            .and_then(|entry| entry.analysis.as_ref())
            .and_then(|result| result.as_ref().ok())
            .map(|stamped| stamped.analysis.base.is_some() as usize + stamped.analysis.copies.len())
            .unwrap_or(0);
        if files == 0 {
            self.selected_file = 0;
        } else {
            self.selected_file = self.selected_file.min(files - 1);
        }
    }

    pub(super) fn selected_entry<'a>(&self, session: &'a Session) -> Option<&'a GroupEntry> {
        session.groups.get(self.selected_group)
    }

    pub(super) fn selected_view_path<'a>(
        &self,
        session: &'a Session,
    ) -> Option<&'a std::path::Path> {
        if self.tab != Tab::Files {
            return None;
        }
        let analysis = self
            .selected_entry(session)
            .and_then(|entry| entry.analysis.as_ref())
            .and_then(|result| result.as_ref().ok())?;
        view_paths(&analysis.analysis)
            .get(self.selected_file)
            .copied()
    }

    pub(super) fn actions(
        &self,
        session: &Session,
        config: &Config,
        tools: &ExternalTools,
    ) -> Vec<ActionDescriptor> {
        let pending = pending_work(session);
        let mut actions = self
            .selected_entry(session)
            .and_then(|entry| entry.analysis.as_ref())
            .and_then(|result| result.as_ref().ok())
            .map(|stamped| actions_for_with_pending(&stamped.analysis, config, tools, pending))
            .unwrap_or_default();
        if self.tab != Tab::Files {
            actions.retain(|action| {
                !matches!(
                    action.kind,
                    deconflict_core::actions::ActionKind::Tool { .. }
                        | deconflict_core::actions::ActionKind::ViewImages
                )
            });
        }
        if actions.is_empty() {
            actions = quit_actions(pending);
        }
        actions
    }

    pub(super) fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > 200 {
            let drain = self.log.len() - 200;
            self.log.drain(..drain);
        }
    }

    pub(super) fn take_pending_tool(&mut self) -> Option<(std::path::PathBuf, Vec<ToolCommand>)> {
        self.pending_tool.take()
    }

    fn select_group(&mut self, index: usize, session: &Session) {
        if index >= session.groups.len() {
            return;
        }
        self.selected_group = index;
        self.selected_file = 0;
        self.show_config = false;
        self.diff_text = None;
        self.push_log(format!(
            "conflict {}: {}",
            index + 1,
            session.groups[index].label
        ));
    }

    fn advance(&mut self, direction: isize, session: &Session) {
        if session.groups.is_empty() {
            return;
        }
        let count = session.groups.len() as isize;
        let mut index = self.selected_group as isize;
        for _ in 0..count {
            index = (index + direction + count) % count;
            if !session.groups[index as usize].is_decided() {
                self.select_group(index as usize, session);
                return;
            }
        }
        self.push_log("all conflicts decided");
    }

    pub(super) fn visible_tabs(&self, session: &Session) -> Vec<Tab> {
        let mut tabs = Vec::new();
        if self.selected_entry(session).is_some() {
            tabs.extend([Tab::Meta, Tab::Files, Tab::Diff]);
            if self
                .selected_entry(session)
                .and_then(|entry| entry.analysis.as_ref())
                .and_then(|result| result.as_ref().ok())
                .is_some_and(|stamped| stamped.analysis.kind == "image")
            {
                tabs.push(Tab::Images);
            }
        }
        tabs.push(Tab::Log);
        let status = session.status();
        if status.errors > 0
            || session
                .groups
                .iter()
                .any(|entry| matches!(entry.analysis, Some(Err(_))))
        {
            tabs.push(Tab::Errors);
        }
        tabs.push(Tab::Help);
        tabs
    }

    pub(super) fn cycle_tab(&mut self, session: &Session, direction: isize) {
        let tabs = self.visible_tabs(session);
        if tabs.is_empty() {
            return;
        }
        let index = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0) as isize;
        let next = (index + direction).rem_euclid(tabs.len() as isize) as usize;
        self.tab = tabs[next];
    }

    pub(super) fn ensure_visible_tab(&mut self, session: &Session) {
        if !self.visible_tabs(session).contains(&self.tab) {
            self.tab = Tab::Log;
        }
    }

    pub(super) fn image_pair(
        &mut self,
        session: &Session,
    ) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        if self.tab != Tab::Images || self.show_config {
            self.last_image_request = None;
            return None;
        }
        let entry = self.selected_entry(session)?;
        let analysis = entry.analysis.as_ref()?.as_ref().ok()?;
        let pair = (
            analysis.analysis.base.as_ref()?.path.clone(),
            analysis.analysis.copies.first()?.path.clone(),
        );
        if self.last_image_request.as_ref() == Some(&pair) {
            return None;
        }
        self.last_image_request = Some(pair.clone());
        Some(pair)
    }

    pub(super) fn handle_key(
        &mut self,
        key: KeyEvent,
        session: &mut Session,
        config: &Config,
        tools: &ExternalTools,
    ) -> Result<(), String> {
        let actions = self.actions(session, config, tools);
        let character = match key.code {
            KeyCode::Char(value) => Some(value),
            _ => None,
        };

        if character == Some('Q') && !actions.iter().any(|action| action.key == "Q") {
            self.quit(session);
            return Ok(());
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => {
                    self.interrupt(session);
                    return Ok(());
                }
                KeyCode::Char('t') => {
                    self.cycle_tab(session, 1);
                    return Ok(());
                }
                KeyCode::Up => {
                    self.advance(-1, session);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.advance(1, session);
                    return Ok(());
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Tab => {
                self.cycle_tab(session, 1);
                return Ok(());
            }
            KeyCode::BackTab => {
                self.cycle_tab(session, -1);
                return Ok(());
            }
            KeyCode::Char('h') => {
                self.tab = Tab::Help;
                return Ok(());
            }
            KeyCode::Char('?') => {
                self.tab = Tab::Diff;
                self.show_config = true;
                return Ok(());
            }
            KeyCode::Esc => {
                self.interrupt(session);
                return Ok(());
            }
            KeyCode::Enter => {
                if let Some(action) = actions.iter().find(|action| action.key == "Enter") {
                    return self.activate(action, session, config, tools);
                }
            }
            KeyCode::Up => {
                if self.tab == Tab::Files {
                    self.selected_file = self.selected_file.saturating_sub(1);
                } else {
                    self.advance(-1, session);
                }
                return Ok(());
            }
            KeyCode::Down => {
                if self.tab == Tab::Files {
                    let files = self
                        .selected_entry(session)
                        .and_then(|entry| entry.analysis.as_ref())
                        .and_then(|result| result.as_ref().ok())
                        .map(|stamped| {
                            stamped.analysis.base.is_some() as usize + stamped.analysis.copies.len()
                        })
                        .unwrap_or(0);
                    if files > 0 {
                        self.selected_file = self.selected_file.saturating_add(1).min(files - 1);
                    }
                } else {
                    self.advance(1, session);
                }
                return Ok(());
            }
            KeyCode::Home => {
                self.select_group(0, session);
                return Ok(());
            }
            KeyCode::End => {
                if !session.groups.is_empty() {
                    self.select_group(session.groups.len() - 1, session);
                }
                return Ok(());
            }
            _ => {}
        }

        if let Some(character) = character {
            if let Some(action) = actions
                .iter()
                .find(|action| action.key == character.to_string())
            {
                return self.activate(action, session, config, tools);
            }
        }
        Ok(())
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        session: &mut Session,
        config: &Config,
        tools: &ExternalTools,
    ) -> Result<(), String> {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.main_area.contains(point)
                    && point.x + 1 >= self.separator_x
                    && point.x <= self.separator_x + 1
                {
                    self.dragging_split = true;
                    return Ok(());
                }
                if let Some(tab) = self
                    .tab_areas
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .map(|(_, tab)| *tab)
                {
                    self.tab = tab;
                    self.show_config = false;
                    return Ok(());
                }
                if let Some(index) = self.group_rows.iter().position(|rect| rect.contains(point)) {
                    self.select_group(index, session);
                    return Ok(());
                }
                if self.tab == Tab::Files {
                    if let Some(index) = self.file_rows.iter().position(|rect| rect.contains(point))
                    {
                        self.selected_file = index;
                        return Ok(());
                    }
                }
                if let Some(index) = self
                    .action_areas
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .map(|(_, index)| *index)
                {
                    let actions = self.actions(session, config, tools);
                    if let Some(action) = actions.get(index) {
                        return self.activate(action, session, config, tools);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging_split => {
                self.split_percent = split_percent(point.x, self.main_area);
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging_split = false,
            _ => {}
        }
        Ok(())
    }

    fn quit(&mut self, session: &mut Session) {
        session.quit();
        self.should_exit = true;
        self.push_log("quit requested; finishing scan before applying recorded choices");
    }

    fn force_quit(&mut self, session: &mut Session) {
        session.force_quit();
        self.should_exit = true;
        self.push_log("quit; cancelling scan and discarding choices");
    }

    fn interrupt(&mut self, session: &mut Session) {
        if pending_work(session) {
            self.force_quit(session);
        } else {
            self.quit(session);
        }
    }

    fn activate(
        &mut self,
        action: &ActionDescriptor,
        session: &mut Session,
        config: &Config,
        tools: &ExternalTools,
    ) -> Result<(), String> {
        use deconflict_core::actions::ActionKind;

        match &action.kind {
            ActionKind::Quit => {
                self.interrupt(session);
                return Ok(());
            }
            ActionKind::ForceQuit => {
                self.quit(session);
                return Ok(());
            }
            ActionKind::Config => {
                self.tab = Tab::Diff;
                self.show_config = true;
                return Ok(());
            }
            _ => {}
        }

        let Some(entry) = self.selected_entry(session) else {
            return Ok(());
        };
        let base = entry.group.base.clone();
        match &action.kind {
            ActionKind::Resolve(decision) => {
                let decision = *decision;
                session.record_decision(&base, decision);
                self.push_log(format!("recorded: {} ({})", action.label, action.key));
                self.advance(1, session);
            }
            ActionKind::Skip => {
                session.record_skip(&base);
                self.push_log("recorded: skip");
                self.advance(1, session);
            }
            ActionKind::Metadata => {
                self.tab = Tab::Meta;
                self.show_config = false;
            }
            ActionKind::ViewImages => {
                self.tab = Tab::Images;
                self.show_config = false;
                self.push_log("images tab: showing inline comparison");
            }
            ActionKind::TextDiff => {
                let Some(analysis) = self.selected_entry(session).and_then(|entry| {
                    entry
                        .analysis
                        .as_ref()
                        .and_then(|result| result.as_ref().ok())
                }) else {
                    return Ok(());
                };
                self.tab = Tab::Diff;
                self.show_config = false;
                self.diff_text = Some(match text_diff(&analysis.analysis) {
                    Ok(diff) => diff,
                    Err(error) => format!("diff unavailable: {error}"),
                });
            }
            ActionKind::Quit | ActionKind::ForceQuit | ActionKind::Config => {
                unreachable!("handled above")
            }
            ActionKind::Tool { .. } => {
                let Some(analysis) = self.selected_entry(session).and_then(|entry| {
                    entry
                        .analysis
                        .as_ref()
                        .and_then(|result| result.as_ref().ok())
                }) else {
                    return Ok(());
                };
                let selected = self
                    .selected_view_path(session)
                    .map(std::path::PathBuf::from);
                let commands =
                    commands_for(&analysis.analysis, action, tools, selected.as_deref())?;
                if commands.is_empty() {
                    self.push_log(format!("{}: no files available", action.label));
                } else {
                    self.pending_tool = Some((base, commands));
                    self.push_log(format!("launching: {}", action.label));
                }
            }
        }
        let _ = config;
        Ok(())
    }

    pub(super) fn run_tool(commands: &[ToolCommand]) -> Vec<String> {
        let mut lines = Vec::new();
        for command in commands {
            let mut process = Command::new(&command.program);
            process.args(&command.args);
            if command.mode == ToolMode::Gui {
                process
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                match process.spawn() {
                    Ok(_) => lines.push(format!(
                        "launched: {} {}",
                        command.program.display(),
                        command
                            .args
                            .iter()
                            .map(|arg| arg.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                    Err(error) => lines.push(format!("tool failed: {error}")),
                }
                continue;
            }
            match process.status() {
                Ok(status) => {
                    lines.push(format!("{} finished: {status}", command.program.display()))
                }
                Err(error) => lines.push(format!("tool failed: {error}")),
            }
        }
        lines
    }

    pub(super) fn finish_tool(
        &mut self,
        base: &std::path::Path,
        lines: Vec<String>,
        session: &mut Session,
    ) -> Result<(), String> {
        for line in lines {
            self.push_log(line);
        }
        session.refresh_analysis(base)?;
        self.sync(session);
        Ok(())
    }

    pub(super) fn recommendation(entry: &GroupEntry) -> Option<String> {
        let analysis = entry.analysis.as_ref()?.as_ref().ok()?.analysis.clone();
        recommendation_label(&analysis, &recommend(&analysis, analysis.kind == "text")?)
    }
}

fn has_pending_work(scan_complete: bool, analysis_complete: bool) -> bool {
    !(scan_complete && analysis_complete)
}

fn pending_work(session: &Session) -> bool {
    has_pending_work(session.scan_complete(), session.analysis_complete())
}

/// Clamp a drag position into a usable main split, in percent.
fn split_percent(column: u16, area: Rect) -> u16 {
    if area.width == 0 {
        return 38;
    }
    let relative = column.saturating_sub(area.x);
    let percent = relative.saturating_mul(100) / area.width;
    percent.clamp(20, 80)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height: 10,
        }
    }

    #[test]
    fn split_drag_is_clamped() {
        assert_eq!(split_percent(0, area(100)), 20);
        assert_eq!(split_percent(40, area(100)), 40);
        assert_eq!(split_percent(99, area(100)), 80);
        assert_eq!(split_percent(50, area(0)), 38);
    }

    #[test]
    fn tab_cycles_in_both_directions() {
        assert_eq!(Tab::default(), Tab::Log);
        assert_eq!(Tab::Help.next(), Tab::Meta);
        assert_eq!(Tab::Meta.previous(), Tab::Help);
    }

    #[test]
    fn pending_work_requires_scan_and_analysis_completion() {
        assert!(has_pending_work(false, false));
        assert!(has_pending_work(true, false));
        assert!(has_pending_work(false, true));
        assert!(!has_pending_work(true, true));
    }
}
