//! UI-free action availability and external command construction.

use crate::{
    analysis::{recommend, GroupAnalysis, Recommendation},
    config::Config,
    resolve::Decision,
    tools::{ToolMode, Tools},
};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub enum ActionKind {
    Resolve(Decision),
    Skip,
    Quit,
    ForceQuit,
    TextDiff,
    Metadata,
    Config,
    Tool {
        operation: ToolOperation,
        tool_name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOperation {
    View,
    Edit,
    Diff,
    EditMeta,
}

#[derive(Clone, Debug)]
pub struct ActionDescriptor {
    pub key: String,
    pub label: String,
    pub kind: ActionKind,
    pub mode: Option<ToolMode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub mode: ToolMode,
}

pub fn actions_for(
    analysis: &GroupAnalysis,
    config: &Config,
    tools: &Tools,
) -> Vec<ActionDescriptor> {
    let mut actions = Vec::new();
    if let Some(recommendation) = recommend(analysis, analysis.kind == "text") {
        let label = match &recommendation {
            Recommendation::Base => "keep base".to_string(),
            Recommendation::Copy(index) => format!("keep copy #{}", index + 1),
        };
        let decision = match recommendation {
            Recommendation::Base => Decision::KeepBase,
            Recommendation::Copy(index) => Decision::KeepCopy(index),
        };
        actions.push(action("Enter", &label, ActionKind::Resolve(decision), None));
    }
    if analysis.base.is_some() {
        actions.push(action(
            "0",
            "keep base",
            ActionKind::Resolve(Decision::KeepBase),
            None,
        ));
    }
    for (index, _) in analysis.copies.iter().enumerate() {
        actions.push(action(
            &(index + 1).to_string(),
            &format!("keep copy #{}", index + 1),
            ActionKind::Resolve(Decision::KeepCopy(index)),
            None,
        ));
    }
    actions.push(action(
        "h",
        "keep both",
        ActionKind::Resolve(Decision::KeepBoth),
        None,
    ));
    actions.push(action("s", "skip", ActionKind::Skip, None));

    add_file_actions(&mut actions, analysis, config, tools);

    actions.push(action("m", "metadata", ActionKind::Metadata, None));
    actions.push(action("?", "tools/config", ActionKind::Config, None));
    actions.push(action("q", "quit after scan", ActionKind::Quit, None));
    actions.push(action("Q", "force quit", ActionKind::ForceQuit, None));
    actions
}

fn add_file_actions(
    actions: &mut Vec<ActionDescriptor>,
    analysis: &GroupAnalysis,
    config: &Config,
    tools: &Tools,
) {
    let kind = analysis.kind.as_str();
    let has_sibling =
        (analysis.base.is_some() && !analysis.copies.is_empty()) || analysis.copies.len() > 1;
    if kind == "text" && has_sibling {
        actions.push(action("d", "diff", ActionKind::TextDiff, None));
    }

    let view = view_tool(config, kind, tools);
    let edit = edit_tool(config, kind, tools);
    let collapsed = view
        .as_ref()
        .zip(edit.as_ref())
        .is_some_and(|(view, edit)| tools.get(view) == tools.get(edit));

    if kind == "kdbx" {
        if has_sibling {
            if let Some(tool_name) = view {
                add_tool(actions, "v", "merge", ToolOperation::View, tool_name, tools);
            }
        }
        return;
    }

    if let Some(tool_name) = view.filter(|_| !collapsed) {
        add_tool(
            actions,
            view_key(kind),
            view_label(kind),
            ToolOperation::View,
            tool_name,
            tools,
        );
    }
    if let Some(tool_name) = edit {
        add_tool(
            actions,
            "e",
            edit_label(kind),
            ToolOperation::Edit,
            tool_name,
            tools,
        );
    }

    if kind == "text" || kind == "other" {
        if has_sibling && tools.available("diff") {
            let mode = tools.mode("diff");
            let (key, label) = match mode {
                ToolMode::PlainTerminal => ("x", "external diff"),
                ToolMode::Tui => ("t", "TUI diff"),
                ToolMode::Gui => ("g", "GUI diff"),
            };
            add_tool(
                actions,
                key,
                label,
                ToolOperation::Diff,
                "diff".into(),
                tools,
            );
        }
    } else if kind == "office" {
        let tool_name = if tools.available("office_compare") {
            Some("office_compare")
        } else if tools.available("office") {
            Some("office")
        } else if tools.available("exiftool") {
            Some("exiftool")
        } else {
            None
        };
        if has_sibling {
            if let Some(tool_name) = tool_name {
                add_tool(
                    actions,
                    "d",
                    "diff",
                    ToolOperation::Diff,
                    tool_name.into(),
                    tools,
                );
            }
        }
    } else if matches!(kind, "audio" | "image" | "video")
        && has_sibling
        && tools.available("exiftool")
    {
        add_tool(
            actions,
            "d",
            "metadata diff",
            ToolOperation::Diff,
            "exiftool".into(),
            tools,
        );
    }

    if kind == "image" && tools.available("exiftool") {
        add_tool(
            actions,
            "x",
            "edit metadata",
            ToolOperation::EditMeta,
            "exiftool".into(),
            tools,
        );
    }
}

fn action(key: &str, label: &str, kind: ActionKind, mode: Option<ToolMode>) -> ActionDescriptor {
    ActionDescriptor {
        key: key.to_string(),
        label: label.to_string(),
        kind,
        mode,
    }
}

fn add_tool(
    actions: &mut Vec<ActionDescriptor>,
    key: &str,
    label: &str,
    operation: ToolOperation,
    tool_name: String,
    tools: &Tools,
) {
    actions.push(action(
        key,
        label,
        ActionKind::Tool {
            operation,
            tool_name: tool_name.clone(),
        },
        Some(tools.mode(&tool_name)),
    ));
}

fn config_tool(section: &toml::Table, kind: &str, fallback: &str) -> String {
    section
        .get(kind)
        .and_then(toml::Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn view_tool(config: &Config, kind: &str, tools: &Tools) -> Option<String> {
    let tool = view_tool_name(config, kind)?;
    if tools.available(&tool) {
        Some(tool)
    } else if matches!(kind, "audio" | "image" | "video") && tools.available("viewer") {
        Some("viewer".into())
    } else {
        None
    }
}

fn view_tool_name(config: &Config, kind: &str) -> Option<String> {
    let fallback = match kind {
        "text" | "other" => "editor",
        "audio" => "audio_player",
        "image" => "image_viewer",
        "video" => "viewer",
        "office" => "office",
        "kdbx" => "keepass",
        _ => return None,
    };
    Some(config_tool(&config.file_types, kind, fallback))
}

fn edit_tool(config: &Config, kind: &str, tools: &Tools) -> Option<String> {
    let tool = edit_tool_name(config, kind)?;
    tools.available(&tool).then_some(tool)
}

fn edit_tool_name(config: &Config, kind: &str) -> Option<String> {
    let fallback = match kind {
        "text" | "other" => "editor",
        "audio" => "mp3_editor",
        "image" => "image_editor",
        "office" => "office",
        _ => return None,
    };
    Some(config_tool(&config.file_types_edit, kind, fallback))
}

pub fn tool_names_for_kind(kind: &str, config: &Config) -> Vec<String> {
    let mut names = Vec::new();
    for name in [view_tool_name(config, kind), edit_tool_name(config, kind)]
        .into_iter()
        .flatten()
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let related = match kind {
        "text" | "other" => &["diff"][..],
        "audio" | "image" | "video" => &["exiftool", "viewer"][..],
        "office" => &["office_compare", "exiftool"][..],
        _ => &[][..],
    };
    for name in related {
        let name = (*name).to_string();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn view_key(kind: &str) -> &str {
    if kind == "office" {
        "o"
    } else {
        "v"
    }
}

fn view_label(kind: &str) -> &str {
    match kind {
        "audio" => "listen",
        "video" => "play",
        "office" => "open",
        _ => "view",
    }
}

fn edit_label(kind: &str) -> &str {
    match kind {
        "audio" => "edit tags",
        "video" => "edit",
        _ => "edit",
    }
}

pub fn commands_for(
    analysis: &GroupAnalysis,
    action: &ActionDescriptor,
    tools: &Tools,
) -> Result<Vec<ToolCommand>, String> {
    let ActionKind::Tool {
        operation,
        tool_name,
    } = &action.kind
    else {
        return Ok(Vec::new());
    };
    let program = tools
        .get(tool_name)
        .filter(|_| tools.available(tool_name))
        .ok_or_else(|| format!("configured tool '{tool_name}' is unavailable"))?
        .to_path_buf();
    let mode = tools.mode(tool_name);
    let paths = analysis_paths(analysis);
    let target = analysis
        .copies
        .first()
        .map(|file| file.path.as_path())
        .or_else(|| analysis.base.as_ref().map(|file| file.path.as_path()))
        .ok_or_else(|| "no file is available for this action".to_string())?;
    let sibling = paths.iter().copied().find(|path| *path != target);

    let mut commands = Vec::new();
    match operation {
        ToolOperation::View if analysis.kind == "kdbx" => {
            if let Some(sibling) = sibling {
                commands.push(tool_command(
                    &program,
                    mode,
                    [
                        "merge".into(),
                        sibling.as_os_str().to_os_string(),
                        target.as_os_str().to_os_string(),
                    ],
                ));
            }
        }
        ToolOperation::View if matches!(analysis.kind.as_str(), "text" | "other" | "office") => {
            let mut args = vec![target.as_os_str().to_os_string()];
            if let Some(sibling) = sibling {
                args.push(sibling.as_os_str().to_os_string());
            }
            commands.push(tool_command(&program, mode, args));
        }
        ToolOperation::View => {
            for path in std::iter::once(target).chain(paths.into_iter().filter(|p| *p != target)) {
                commands.push(tool_command(
                    &program,
                    mode,
                    [path.as_os_str().to_os_string()],
                ));
            }
        }
        ToolOperation::Edit if matches!(analysis.kind.as_str(), "text" | "other" | "office") => {
            let mut args = vec![target.as_os_str().to_os_string()];
            if let Some(sibling) = sibling {
                args.push(sibling.as_os_str().to_os_string());
            }
            commands.push(tool_command(&program, mode, args));
        }
        ToolOperation::Edit | ToolOperation::EditMeta => commands.push(tool_command(
            &program,
            mode,
            [target.as_os_str().to_os_string()],
        )),
        ToolOperation::Diff if matches!(analysis.kind.as_str(), "text" | "other") => {
            if let Some(sibling) = sibling {
                commands.push(tool_command(
                    &program,
                    mode,
                    [
                        sibling.as_os_str().to_os_string(),
                        target.as_os_str().to_os_string(),
                    ],
                ));
            }
        }
        ToolOperation::Diff if analysis.kind == "office" && tool_name != "exiftool" => {
            if let Some(sibling) = sibling {
                commands.push(tool_command(
                    &program,
                    mode,
                    [
                        "--compare".into(),
                        target.as_os_str().to_os_string(),
                        sibling.as_os_str().to_os_string(),
                    ],
                ));
            }
        }
        ToolOperation::Diff => {
            if let Some(sibling) = sibling {
                commands.push(tool_command(
                    &program,
                    mode,
                    [
                        "-diff".into(),
                        sibling.as_os_str().to_os_string(),
                        target.as_os_str().to_os_string(),
                        "--system:all".into(),
                        "-s".into(),
                        "-a".into(),
                    ],
                ));
            }
        }
    }
    Ok(commands)
}

fn analysis_paths(analysis: &GroupAnalysis) -> Vec<&Path> {
    let mut paths = Vec::with_capacity(analysis.copies.len() + 1);
    if let Some(base) = &analysis.base {
        paths.push(base.path.as_path());
    }
    paths.extend(analysis.copies.iter().map(|file| file.path.as_path()));
    paths
}

fn tool_command(
    program: &Path,
    mode: ToolMode,
    args: impl IntoIterator<Item = OsString>,
) -> ToolCommand {
    ToolCommand {
        program: program.to_path_buf(),
        args: args.into_iter().collect(),
        mode,
    }
}

pub fn text_diff(analysis: &GroupAnalysis) -> io::Result<String> {
    let Some(base) = &analysis.base else {
        return Ok("nothing to diff: no base file\n".into());
    };
    let Some(copy) = analysis.copies.first() else {
        return Ok("nothing to diff: no copy file\n".into());
    };
    let base_text = std::fs::read_to_string(&base.path)?;
    let copy_text = std::fs::read_to_string(&copy.path)?;
    let base_lines: Vec<_> = base_text.lines().collect();
    let copy_lines: Vec<_> = copy_text.lines().collect();
    let prefix = base_lines
        .iter()
        .zip(&copy_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = base_lines[prefix..]
        .iter()
        .rev()
        .zip(copy_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    if prefix == base_lines.len() && prefix == copy_lines.len() {
        return Ok("no textual difference\n".into());
    }
    let context_before = prefix.min(3);
    let context_after = suffix.min(3);
    let start = prefix - context_before;
    let base_end = base_lines.len() - suffix + context_after;
    let copy_end = copy_lines.len() - suffix + context_after;
    let mut output = format!(
        "--- {}\n+++ {}\n@@ -{},{} +{},{} @@\n",
        base.path.display(),
        copy.path.display(),
        start + 1,
        base_end - start,
        start + 1,
        copy_end - start
    );
    for line in &base_lines[start..prefix] {
        output.push_str(&format!(" {line}\n"));
    }
    for line in &base_lines[prefix..base_lines.len() - suffix] {
        output.push_str(&format!("-{line}\n"));
    }
    for line in &copy_lines[prefix..copy_lines.len() - suffix] {
        output.push_str(&format!("+{line}\n"));
    }
    for line in &base_lines[base_lines.len() - context_after..] {
        output.push_str(&format!(" {line}\n"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;
    use std::{collections::HashMap, time::SystemTime};

    fn analysis(kind: &str, base: &Path, copy: &Path) -> GroupAnalysis {
        let file = |path: &Path| FileInfo {
            path: path.to_path_buf(),
            size: 1,
            modified: SystemTime::UNIX_EPOCH,
            sha256: String::new(),
            is_text: true,
        };
        GroupAnalysis {
            base: Some(file(base)),
            copies: vec![file(copy)],
            all_equal: false,
            all_text: true,
            kind: kind.into(),
            meta: vec![None],
            meta_diff: vec![None],
            meta_fields: vec![],
        }
    }

    fn available_tools(names: &[&str]) -> Tools {
        let path = std::env::current_exe().unwrap();
        Tools {
            found: names
                .iter()
                .map(|name| ((*name).into(), path.clone()))
                .collect::<HashMap<_, _>>(),
        }
    }

    #[test]
    fn text_actions_keep_builtin_diff_and_only_offer_detected_tools() {
        let base = Path::new("base.txt");
        let copy = Path::new("copy.txt");
        let analysis = analysis("text", base, copy);
        let config = Config::default();
        let actions = actions_for(&analysis, &config, &available_tools(&["editor", "diff"]));
        let keys: Vec<_> = actions.iter().map(|action| action.key.as_str()).collect();

        assert!(keys.contains(&"d"));
        assert!(keys.contains(&"x"));
        assert!(keys.contains(&"e"));
        assert!(keys.contains(&"m"));
        assert!(!keys.contains(&"v"));
        assert!(actions.iter().any(|action| action.key == "Q"));

        let without_tools = actions_for(&analysis, &config, &Tools::default());
        assert!(!without_tools
            .iter()
            .any(|action| matches!(action.kind, ActionKind::Tool { .. })));
    }

    #[test]
    fn external_text_diff_command_uses_sibling_then_selected_copy() {
        let base = Path::new("/scan/base.txt");
        let copy = Path::new("/scan/copy.txt");
        let analysis = analysis("text", base, copy);
        let tools = available_tools(&["editor", "diff"]);
        let action = actions_for(&analysis, &Config::default(), &tools)
            .into_iter()
            .find(|action| action.key == "x")
            .unwrap();
        let commands = commands_for(&analysis, &action, &tools).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args,
            [
                base.as_os_str().to_os_string(),
                copy.as_os_str().to_os_string()
            ]
        );
    }

    #[test]
    fn comparisons_and_merges_require_a_sibling_file() {
        let copy = Path::new("/scan/copy.txt");
        let mut analysis = analysis("text", Path::new("/scan/base.txt"), copy);
        analysis.base = None;
        let tools = available_tools(&["editor", "diff", "keepass", "exiftool"]);
        let config = Config::default();

        assert!(!actions_for(&analysis, &config, &tools)
            .iter()
            .any(|action| matches!(action.kind, ActionKind::TextDiff)));

        analysis.kind = "kdbx".into();
        assert!(!actions_for(&analysis, &config, &tools)
            .iter()
            .any(|action| matches!(
                action.kind,
                ActionKind::Tool {
                    operation: ToolOperation::View,
                    ..
                }
            )));
    }

    #[test]
    fn media_view_opens_the_selected_copy_before_the_base() {
        let base = Path::new("/scan/base.jpg");
        let copy = Path::new("/scan/copy.jpg");
        let analysis = analysis("image", base, copy);
        let tools = available_tools(&["viewer"]);
        let action = actions_for(&analysis, &Config::default(), &tools)
            .into_iter()
            .find(|action| action.key == "v")
            .unwrap();
        let commands = commands_for(&analysis, &action, &tools).unwrap();

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].args, [copy.as_os_str().to_os_string()]);
        assert_eq!(commands[1].args, [base.as_os_str().to_os_string()]);
    }

    #[test]
    fn video_actions_do_not_offer_editing() {
        let analysis = analysis(
            "video",
            Path::new("/scan/base.mp4"),
            Path::new("/scan/copy.mp4"),
        );
        let tools = available_tools(&["editor", "viewer", "exiftool"]);
        assert!(!actions_for(&analysis, &Config::default(), &tools)
            .iter()
            .any(|action| matches!(
                action.kind,
                ActionKind::Tool {
                    operation: ToolOperation::Edit,
                    ..
                }
            )));
    }

    #[test]
    fn text_diff_renders_changed_lines_and_equal_files() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base.txt");
        let copy = temp.path().join("copy.txt");
        std::fs::write(&base, "one\nold\nsame\n").unwrap();
        std::fs::write(&copy, "one\nnew\nsame\n").unwrap();
        let analysis = analysis("text", &base, &copy);
        let diff = text_diff(&analysis).unwrap();
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));

        std::fs::write(&copy, "one\nold\nsame\n").unwrap();
        assert_eq!(text_diff(&analysis).unwrap(), "no textual difference\n");
    }
}
