//! Full-screen Ratatui frontend. Process startup/dispatch stays in `main.rs`.

mod app;
mod images;
mod session;
mod views;

use crate::cli::Cli;
use crate::cli_config::{
    detected_tools, effective_scan, general_backup, general_cache, media_tools,
};
use crate::progress::ScanProgress;
use app::App;
use crossterm::{
    event::{self, Event as TermEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use deconflict_core::{config::Config, tools::ToolMode};
use ratatui::{backend::CrosstermBackend, Terminal};
use session::{Session, SessionOptions, SessionRequest};
use std::{
    io::{self, IsTerminal, Stdout, Write},
    path::PathBuf,
    time::Duration,
};

type TerminalHandle = Terminal<CrosstermBackend<Stdout>>;

pub(crate) struct TuiOptions<'a> {
    pub(crate) dirs: &'a [PathBuf],
    pub(crate) pattern: Option<&'a str>,
    pub(crate) no_cache: bool,
    pub(crate) refresh: bool,
    pub(crate) dry_run: bool,
    pub(crate) show_progress: bool,
}

pub(crate) fn run(cli: &Cli, options: TuiOptions<'_>) -> Result<(), String> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        return Err("the TUI requires an interactive terminal; use --no-tui".into());
    }
    let (scan_config, patterns, roots, config) =
        effective_scan(cli.config.as_deref(), cli, options.dirs, options.pattern)?;
    let backup = general_backup(&config);
    let cache_dir = general_cache(&config);
    let media = media_tools(&config);
    let external_tools = detected_tools(&config);

    let mut session = Session::start(SessionRequest {
        roots: &roots,
        scan_config: &scan_config,
        patterns: &patterns,
        backup: &backup,
        cache_dir: &cache_dir,
        media: &media,
        options: SessionOptions {
            no_cache: options.no_cache,
            refresh: options.refresh,
            dry_run: options.dry_run,
        },
    })?;

    let mut terminal = enter_terminal()?;
    let mut app = App::new();
    let loop_result = event_loop(
        &mut terminal,
        &mut app,
        &mut session,
        &config,
        &external_tools,
    );
    let wait_with_progress = !(session.scan_complete() && session.analysis_complete())
        && !session.force_quit_requested();
    leave_terminal(&mut terminal)?;
    loop_result?;

    if wait_with_progress {
        let progress = ScanProgress::new(options.show_progress, session.estimated_file_count())
            .without_finish_message();
        while !(session.scan_complete() && session.analysis_complete())
            && session.worker_error().is_none()
        {
            session.poll(Duration::from_millis(50));
            let status = session.status();
            let (analyses_done, analyses_queued) = session.group_analysis_counts();
            progress.update_display(
                status.files,
                status.directories,
                analyses_done,
                analyses_queued,
                status.current_path.as_deref(),
                session.elapsed(),
            );
        }
        progress.finish_display("");
    }

    let summary = session.finish()?;
    for line in summary
        .lines
        .iter()
        .filter(|line| !line.starts_with("moved:") && !line.starts_with("would move:"))
    {
        println!("\r\n{line}\r\n");
    }
    Ok(())
}

fn emit_inline_images(
    _terminal: &TerminalHandle,
    app: &mut App,
    session: &mut Session,
) -> Result<(), String> {
    if !images::kitty_supported() {
        return Ok(());
    }
    let had_request = app.last_image_request.is_some();
    let pair = app.image_pair(session);
    let escape = if had_request && app.last_image_request.is_none() {
        Some(images::clear_escape())
    } else if let (Some((left, right)), Some(area)) = (pair, app.image_area) {
        let row = area.y.saturating_add(1);
        let column = area.x.saturating_add(1);
        images::pair_escape(&left, &right, area.width, area.height)
            .map(|escape| format!("\x1b[s\x1b[{row};{column}H{escape}\x1b[u"))
    } else {
        None
    };
    if let Some(escape) = escape {
        let mut stdout = io::stdout();
        stdout
            .write_all(escape.as_bytes())
            .map_err(|error| format!("cannot emit inline images: {error}"))?;
        stdout
            .flush()
            .map_err(|error| format!("cannot flush inline images: {error}"))?;
    }
    Ok(())
}

fn event_loop(
    terminal: &mut TerminalHandle,
    app: &mut App,
    session: &mut Session,
    config: &Config,
    tools: &deconflict_core::tools::Tools,
) -> Result<(), String> {
    loop {
        let scanning = !(session.scan_complete() && session.analysis_complete());
        let input_timeout = if scanning {
            Duration::from_millis(10)
        } else {
            Duration::from_millis(100)
        };
        if event::poll(input_timeout)
            .map_err(|error| format!("cannot poll terminal events: {error}"))?
        {
            let mut pending = true;
            while pending {
                match event::read()
                    .map_err(|error| format!("cannot read terminal event: {error}"))?
                {
                    TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                        app.handle_key(key, session, config, tools)?;
                    }
                    TermEvent::Mouse(mouse) => app.handle_mouse(mouse, session, config, tools)?,
                    TermEvent::Resize(_, _) => terminal
                        .clear()
                        .map_err(|error| format!("cannot clear after resize: {error}"))?,
                    _ => {}
                }
                pending = event::poll(Duration::ZERO)
                    .map_err(|error| format!("cannot poll terminal events: {error}"))?;
            }
        }
        session.poll(Duration::ZERO);
        app.sync(session);
        if let Some(error) = session.worker_error() {
            app.push_log(format!("error: {error}"));
            app.should_exit = true;
        }
        terminal
            .draw(|frame| views::draw(frame, app, session, config, tools))
            .map_err(|error| format!("cannot draw TUI: {error}"))?;
        emit_inline_images(terminal, app, session)?;

        if let Some((base, commands)) = app.take_pending_tool() {
            let capture_output = !commands.is_empty()
                && commands
                    .iter()
                    .all(|command| command.mode == ToolMode::PlainTerminal);
            if !capture_output {
                leave_terminal(terminal)?;
            }
            let lines = App::run_tool(&commands);
            if !capture_output {
                suspend_terminal(terminal)?;
            }
            app.finish_tool(&base, lines, session, capture_output)?;
        }

        if app.should_exit {
            return Ok(());
        }
    }
}

fn enter_terminal() -> Result<TerminalHandle, String> {
    enable_raw_mode().map_err(|error| format!("cannot enable raw mode: {error}"))?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    ) {
        let _ = disable_raw_mode();
        return Err(format!("cannot enter alternate screen: {error}"));
    }
    match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            Err(format!("cannot initialize TUI: {error}"))
        }
    }
}

/// Leave the alternate screen so blocking external tools can use the terminal.
fn leave_terminal(terminal: &mut TerminalHandle) -> Result<(), String> {
    disable_raw_mode().map_err(|error| format!("cannot disable raw mode: {error}"))?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )
    .map_err(|error| format!("cannot leave alternate screen: {error}"))?;
    terminal
        .show_cursor()
        .map_err(|error| format!("cannot show cursor: {error}"))
}

fn suspend_terminal(terminal: &mut TerminalHandle) -> Result<(), String> {
    enable_raw_mode().map_err(|error| format!("cannot re-enable raw mode: {error}"))?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .map_err(|error| format!("cannot re-enter alternate screen: {error}"))?;
    terminal
        .clear()
        .map_err(|error| format!("cannot clear restored TUI: {error}"))
}
