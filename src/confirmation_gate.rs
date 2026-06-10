use std::io::IsTerminal;

use crate::{
    cli,
    cli_commands::{self, keys::KeysCommand},
    cli_effects, operator_templates,
};

pub(crate) fn apply(action: &mut cli::CliAction) -> anyhow::Result<()> {
    match cli_effects::confirmation_outcome(action, std::io::stdin().is_terminal()) {
        cli_effects::ConfirmationOutcome::Allowed => Ok(()),
        cli_effects::ConfirmationOutcome::Denied {
            exit_code,
            reason_code,
        } => {
            eprintln!("Reason code: {reason_code}");
            eprintln!("Confirmation is required for this command. Re-run with --yes or --dry-run.");
            std::process::exit(exit_code);
        }
        cli_effects::ConfirmationOutcome::PromptRequired { reason_code } => {
            if prompt_for_confirmation(reason_code)? {
                apply_confirmed_transition(action);
                Ok(())
            } else {
                eprintln!("Reason code: {reason_code}");
                std::process::exit(3);
            }
        }
    }
}

pub(crate) fn apply_confirmed_transition(action: &mut cli::CliAction) {
    match action {
        cli::CliAction::InitLocal(options)
            if matches!(
                options.mode,
                operator_templates::InitLocalMode::NeedsConfirmation
            ) =>
        {
            options.mode = operator_templates::InitLocalMode::Write;
        }
        cli::CliAction::ModelsOnboardPlan(options)
            if matches!(
                options.mode,
                cli_commands::models_onboard::ModelsOnboardPlanMode::NeedsConfirmation
            ) =>
        {
            options.mode = cli_commands::models_onboard::ModelsOnboardPlanMode::Apply;
        }
        cli::CliAction::Keys(KeysCommand::Import(options))
            if matches!(
                options.mode,
                cli_commands::keys::KeysImportMode::NeedsConfirmation
            ) =>
        {
            options.mode = cli_commands::keys::KeysImportMode::Apply;
        }
        cli::CliAction::Keys(KeysCommand::Probe(options))
            if matches!(
                options.mode,
                cli_commands::keys::KeysProbeMode::NeedsConfirmation
            ) =>
        {
            options.mode = cli_commands::keys::KeysProbeMode::Apply;
        }
        cli::CliAction::Keys(KeysCommand::Disable(options))
            if matches!(
                options.mode,
                cli_commands::keys::KeysDisableMode::NeedsConfirmation
            ) =>
        {
            options.mode = cli_commands::keys::KeysDisableMode::Apply;
        }
        cli::CliAction::Keys(KeysCommand::Restore(options))
            if matches!(
                options.mode,
                cli_commands::keys::KeysRestoreMode::NeedsConfirmation
            ) =>
        {
            options.mode = cli_commands::keys::KeysRestoreMode::Apply;
        }
        cli::CliAction::Keys(KeysCommand::ProbeApply(
            cli_commands::keys::KeysProbeApplyCommand::Apply(options),
        )) if matches!(
            options.mode,
            cli_commands::keys::KeysProbeApplyMode::NeedsConfirmation
        ) =>
        {
            options.mode = cli_commands::keys::KeysProbeApplyMode::Apply;
        }
        cli::CliAction::ReloadApply(options)
            if matches!(
                options.mode,
                cli_commands::reload::ReloadApplyMode::NeedsConfirmation
            ) =>
        {
            options.mode = cli_commands::reload::ReloadApplyMode::Apply;
        }
        _ => {}
    }
}

fn prompt_for_confirmation(reason_code: &str) -> anyhow::Result<bool> {
    use std::io::{self, Write};

    print!("Reason code: {reason_code}\nThis command has side effects. Continue? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
