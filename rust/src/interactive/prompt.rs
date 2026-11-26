use console::style;
use deconflict_core::{
    analysis::{recommendation_label, GroupAnalysis, Recommendation},
    resolve::Decision,
};
use std::io::{self, BufRead, Write};

pub(super) enum PromptChoice {
    Resolve(Decision),
    Skip,
    Quit,
}

pub(super) fn prompt_choice(
    analysis: &GroupAnalysis,
    recommendation: Option<&Recommendation>,
) -> Result<PromptChoice, String> {
    let default = recommendation
        .and_then(|choice| recommendation_label(analysis, choice))
        .map(|label| format!("(Enter) {label} "));
    loop {
        let mut choices = Vec::new();
        if let Some(default) = &default {
            choices.push(style(default).cyan().bold().to_string());
        }
        if analysis.base.is_some() {
            choices.push("(0) keep base".into());
        }
        choices.extend(
            analysis
                .copies
                .iter()
                .enumerate()
                .map(|(index, _)| format!("({}) keep copy #{}", index + 1, index + 1)),
        );
        choices.extend(["(h) keep both".into(), "(s) skip".into(), "(q) quit".into()]);
        print!("{}: ", choices.join(" "));
        io::stdout()
            .flush()
            .map_err(|e| format!("cannot flush prompt: {e}"))?;

        let mut answer = String::new();
        let bytes = io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| format!("cannot read choice: {e}"))?;
        if bytes == 0 {
            return Err("stdin closed before a resolution choice".into());
        }
        let answer = answer.trim().to_lowercase();
        if answer.is_empty() {
            if let Some(recommendation) = recommendation {
                return Ok(PromptChoice::Resolve(match recommendation {
                    Recommendation::Base => Decision::KeepBase,
                    Recommendation::Copy(index) => Decision::KeepCopy(*index),
                }));
            }
        }
        match answer.as_str() {
            "q" | "quit" | "exit" => return Ok(PromptChoice::Quit),
            "s" | "skip" => return Ok(PromptChoice::Skip),
            "h" | "keep-both" => return Ok(PromptChoice::Resolve(Decision::KeepBoth)),
            "0" if analysis.base.is_some() => return Ok(PromptChoice::Resolve(Decision::KeepBase)),
            value => {
                if let Ok(copy_index) = value.parse::<usize>() {
                    if copy_index > 0 && copy_index <= analysis.copies.len() {
                        return Ok(PromptChoice::Resolve(Decision::KeepCopy(copy_index - 1)));
                    }
                }
                println!("invalid choice");
            }
        }
    }
}
