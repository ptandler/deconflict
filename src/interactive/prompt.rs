use console::style;
use deconflict_core::{
    actions::{self, view_paths, ActionDescriptor, ActionKind, ToolCommand},
    analysis::GroupAnalysis,
    config::Config,
    tools::{ToolMode, Tools},
};
use std::{
    io::{self, BufRead, Write},
    process::{Command, Stdio},
};

pub(super) enum PromptChoice {
    Resolve(deconflict_core::resolve::Decision),
    Skip,
    Quit,
    ForceQuit,
    Reprint,
    Review,
}

enum FileSelection<'a> {
    One(&'a std::path::Path),
    Both,
    Cancel,
}

pub(super) fn prompt_choice(
    analysis: &GroupAnalysis,
    config: &Config,
    tools: &Tools,
    pending_work: bool,
) -> Result<PromptChoice, String> {
    let actions = actions::actions_for_with_pending(analysis, config, tools, pending_work);
    loop {
        let mut resolution = Vec::new();
        let mut inspect = Vec::new();
        for action in &actions {
            let label = format!("({}) {}", action.key, action.label);
            let label = if action.key == "Enter" {
                style(label).cyan().bold().to_string()
            } else {
                label
            };
            if matches!(
                &action.kind,
                ActionKind::Resolve(_)
                    | ActionKind::Skip
                    | ActionKind::Quit
                    | ActionKind::ForceQuit
            ) {
                resolution.push(label);
            } else {
                inspect.push(label);
            }
        }
        println!("Available actions:");
        println!("  resolve: {}", resolution.join("  "));
        if !inspect.is_empty() {
            println!("  inspect: {}", inspect.join("  "));
        }
        print!("Choice: ");
        io::stdout()
            .flush()
            .map_err(|error| format!("cannot flush prompt: {error}"))?;

        let mut answer = String::new();
        let bytes = io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|error| format!("cannot read choice: {error}"))?;
        if bytes == 0 {
            return Err("stdin closed before a resolution choice".into());
        }
        let answer = answer.trim();
        if let Some(choice) = parse_quit_choice(answer, pending_work) {
            return Ok(choice);
        }
        let selected = actions.iter().find(|action| {
            if action.key == "Enter" {
                answer.is_empty()
            } else {
                action.key.eq_ignore_ascii_case(answer)
            }
        });
        let Some(selected) = selected else {
            println!("invalid choice");
            continue;
        };
        match &selected.kind {
            ActionKind::Resolve(decision) => return Ok(PromptChoice::Resolve(*decision)),
            ActionKind::Skip => return Ok(PromptChoice::Skip),
            ActionKind::Quit => return Ok(PromptChoice::Quit),
            ActionKind::ForceQuit => return Ok(PromptChoice::ForceQuit),
            ActionKind::TextDiff => match actions::text_diff(analysis) {
                Ok(diff) => print!("{diff}"),
                Err(error) => eprintln!("text diff failed: {error}"),
            },
            ActionKind::ViewImages => println!(
                "inline images are available in the TUI images tab; use (v) for an external viewer"
            ),
            ActionKind::Config => print_action_map(analysis, &actions, config, tools),
            ActionKind::Metadata => return Ok(PromptChoice::Reprint),
            ActionKind::Tool { .. } => {
                match choose_file(analysis)? {
                    FileSelection::One(path) => {
                        run_tool_action(analysis, selected, tools, Some(path), false)
                    }
                    FileSelection::Both => run_tool_action(analysis, selected, tools, None, true),
                    FileSelection::Cancel => {}
                }
                return Ok(PromptChoice::Review);
            }
        }
    }
}

fn choose_file(analysis: &GroupAnalysis) -> Result<FileSelection<'_>, String> {
    let paths = view_paths(analysis);
    if paths.is_empty() {
        return Err("no files are available for this action".into());
    }
    loop {
        println!("which file?");
        for (index, path) in paths.iter().enumerate() {
            println!(
                "  ({}) {}",
                index + 1,
                path.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        if analysis.kind != "audio" {
            println!("  (b) both");
        }
        println!("  (c) cancel");
        print!("Choice: ");
        io::stdout()
            .flush()
            .map_err(|error| format!("cannot flush prompt: {error}"))?;
        let mut answer = String::new();
        let bytes = io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|error| format!("cannot read choice: {error}"))?;
        if bytes == 0 {
            return Err("stdin closed before choosing a file".into());
        }
        let answer = answer.trim().to_ascii_lowercase();
        if answer == "c" || answer == "q" {
            return Ok(FileSelection::Cancel);
        }
        if answer == "b" && analysis.kind != "audio" {
            return Ok(FileSelection::Both);
        }
        if let Ok(index) = answer.parse::<usize>() {
            if let Some(path) = paths.get(index.saturating_sub(1)) {
                return Ok(FileSelection::One(path));
            }
        }
        println!("invalid choice");
    }
}

fn parse_quit_choice(answer: &str, pending_work: bool) -> Option<PromptChoice> {
    if answer == "Q" {
        Some(PromptChoice::Quit)
    } else if matches!(answer.to_ascii_lowercase().as_str(), "q" | "quit" | "exit") {
        Some(if pending_work {
            PromptChoice::ForceQuit
        } else {
            PromptChoice::Quit
        })
    } else {
        None
    }
}

fn print_action_map(
    analysis: &GroupAnalysis,
    actions: &[ActionDescriptor],
    config: &Config,
    tools: &Tools,
) {
    println!("[file type: {}]", analysis.kind);
    println!("actions and commands:");
    for action in actions {
        let label = format!("({}) {}", action.key, action.label);
        match &action.kind {
            ActionKind::Tool { .. } => match actions::commands_for(analysis, action, tools, None) {
                Ok(commands) if !commands.is_empty() => {
                    let commands = commands
                        .iter()
                        .map(|command| {
                            format!("{} ({})", display_command(command), mode_name(command.mode))
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    println!("  {label} -> {commands}");
                }
                Ok(_) => println!("  {label} -> no matching file for this action"),
                Err(error) => println!("  {label} -> unavailable ({error})"),
            },
            ActionKind::TextDiff => println!("  {label} -> built-in text diff"),
            ActionKind::ViewImages => println!("  {label} -> TUI images tab only"),
            ActionKind::Metadata => println!("  {label} -> built-in metadata report"),
            ActionKind::Config => println!("  {label} -> this action list"),
            _ => println!("  {label} -> no external command"),
        }
    }
    print_config_entry("file_types", &config.file_types, &analysis.kind);
    print_config_entry("file_types_edit", &config.file_types_edit, &analysis.kind);
    for tool_name in actions::tool_names_for_kind(&analysis.kind, config) {
        if let Some(path) = tools.get(&tool_name) {
            println!(
                "{tool_name} = {} ({})",
                path.display(),
                mode_name(tools.mode(&tool_name))
            );
        } else {
            println!("{tool_name} = (not found)");
        }
    }
}

fn print_config_entry(name: &str, table: &toml::Table, kind: &str) {
    match table.get(kind).and_then(toml::Value::as_str) {
        Some(value) => println!("[{name}] {kind} = {value:?}"),
        None => println!("[{name}] {kind} = (built-in default)"),
    }
}

fn run_tool_action(
    analysis: &GroupAnalysis,
    action: &ActionDescriptor,
    tools: &Tools,
    selected: Option<&std::path::Path>,
    both: bool,
) {
    let commands_result = if both {
        let mut commands = Vec::new();
        for path in view_paths(analysis) {
            match actions::commands_for(analysis, action, tools, Some(path)) {
                Ok(mut for_path) => commands.append(&mut for_path),
                Err(error) => {
                    eprintln!("cannot prepare tool action: {error}");
                    return;
                }
            }
        }
        Ok(commands)
    } else {
        actions::commands_for(analysis, action, tools, selected)
    };
    let commands = match commands_result {
        Ok(commands) => commands,
        Err(error) => {
            eprintln!("cannot prepare tool action: {error}");
            return;
        }
    };
    if commands.is_empty() {
        eprintln!("no matching file is available for this action");
        return;
    }
    let mut gui_processes = Vec::new();
    for command in commands {
        println!(
            "launching ({}): {}",
            mode_name(command.mode),
            display_command(&command)
        );
        if command.mode == ToolMode::Gui {
            match Command::new(&command.program)
                .args(&command.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => gui_processes.push((command.program, child)),
                Err(error) => eprintln!("cannot launch {}: {error}", command.program.display()),
            }
        } else {
            match Command::new(&command.program).args(&command.args).status() {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("tool exited with {status}"),
                Err(error) => eprintln!("cannot launch {}: {error}", command.program.display()),
            }
        }
    }
    for (program, mut child) in gui_processes {
        match child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("tool exited with {status}"),
            Err(error) => eprintln!("cannot wait for {}: {error}", program.display()),
        }
    }
}

fn display_command(command: &ToolCommand) -> String {
    std::iter::once(command.program.as_os_str())
        .chain(command.args.iter().map(std::ffi::OsString::as_os_str))
        .map(|part| format!("{:?}", part.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn mode_name(mode: ToolMode) -> &'static str {
    match mode {
        ToolMode::PlainTerminal => "terminal",
        ToolMode::Tui => "TUI",
        ToolMode::Gui => "GUI",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_choices_distinguish_waiting_and_forced_quit() {
        assert!(matches!(
            parse_quit_choice("q", true),
            Some(PromptChoice::ForceQuit)
        ));
        assert!(matches!(
            parse_quit_choice("Q", true),
            Some(PromptChoice::Quit)
        ));
        assert!(matches!(
            parse_quit_choice("Q", false),
            Some(PromptChoice::Quit)
        ));
        assert!(matches!(
            parse_quit_choice("QUIT", false),
            Some(PromptChoice::Quit)
        ));
        assert!(parse_quit_choice("", true).is_none());
    }
}
