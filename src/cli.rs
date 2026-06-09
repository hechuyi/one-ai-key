#[cfg(test)]
use clap::CommandFactory;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    Serve {
        config_path: String,
    },
    InitLocal(crate::operator_templates::InitLocalOptions),
    CheckConfig {
        config_path: PathBuf,
        output: crate::cli_report::OutputFormat,
    },
    RouteExplain(crate::cli_commands::route::RouteExplainOptions),
    ModelsList(crate::cli_commands::models::ModelsListOptions),
    ModelsExplain(crate::cli_commands::models::ModelsExplainOptions),
    ModelsOnboardPlan(crate::cli_commands::models_onboard::ModelsOnboardPlanOptions),
    ReloadStatus(crate::cli_commands::reload::ReloadStatusOptions),
    ReloadDiff(crate::cli_commands::reload::ReloadDiffOptions),
    ReloadApply(crate::cli_commands::reload::ReloadApplyOptions),
    ClientTokensList(crate::cli_commands::client_tokens::ClientTokensListOptions),
    Keys(crate::cli_commands::keys::KeysCommand),
    FailuresTail(crate::cli_commands::failures::FailureTailOptions),
    FailuresExplain(crate::cli_commands::failures::FailureExplainOptions),
    Doctor(crate::cli_commands::doctor::DoctorOptions),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorConnectionOptions {
    pub management_url: Option<String>,
    pub deprecated_base_url: Option<String>,
    pub management_token_env: Option<String>,
    pub management_token_stdin: bool,
    pub timeout_seconds: u64,
}

#[derive(Debug, Parser)]
#[command(name = "one-ai-key")]
#[command(about = "Lightweight AI API key router")]
struct Cli {
    #[arg(
        long,
        env = "KEY_POOL_ROUTER_CONFIG",
        default_value = "config/router.yaml",
        global = true,
        hide_env_values = true
    )]
    config: String,

    #[arg(
        long,
        global = true,
        value_name = "url",
        help = "Running management API origin or /management base for operator CLI commands. Client /v1 base URLs are rejected; commands use /management/* endpoints and a management token."
    )]
    management_url: Option<String>,

    #[arg(
        long = "base-url",
        global = true,
        value_name = "url",
        help = "Deprecated compatibility alias for --management-url."
    )]
    deprecated_base_url: Option<String>,

    #[arg(
        long,
        global = true,
        value_name = "ENV",
        conflicts_with = "management_token_stdin",
        hide_env_values = true,
        help = "Environment variable containing the management token."
    )]
    management_token_env: Option<String>,

    #[arg(
        long,
        global = true,
        conflicts_with = "management_token_env",
        help = "Read management token from stdin without echoing it."
    )]
    management_token_stdin: bool,

    #[arg(long, global = true, default_value_t = 10)]
    timeout_seconds: u64,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Serve,
    CheckConfig(CheckConfigArgs),
    Init {
        #[command(subcommand)]
        command: InitCommand,
    },
    Route {
        #[command(subcommand)]
        command: RouteCommand,
    },
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },
    Reload {
        #[command(subcommand)]
        command: ReloadCommand,
    },
    ClientTokens {
        #[command(subcommand)]
        command: ClientTokensCommand,
    },
    Keys {
        #[command(subcommand)]
        command: KeysCommandArgs,
    },
    #[command(about = "Read-only runtime doctor")]
    Doctor(DoctorArgs),
    Failures {
        #[command(subcommand)]
        command: FailuresCommand,
    },
}

#[derive(Debug, Subcommand)]
enum InitCommand {
    Local(InitLocalArgs),
}

#[derive(Debug, Subcommand)]
enum RouteCommand {
    Explain(RouteExplainArgs),
}

#[derive(Debug, Subcommand)]
enum ModelsCommand {
    List(ModelsListArgs),
    Explain(ModelsExplainArgs),
    OnboardPlan(ModelsOnboardPlanArgs),
}

#[derive(Debug, Subcommand)]
enum ReloadCommand {
    Status(ReloadStatusArgs),
    Diff(ReloadDiffArgs),
    Apply(ReloadApplyArgs),
}

#[derive(Debug, Subcommand)]
enum ClientTokensCommand {
    List(ClientTokensListArgs),
}

#[derive(Debug, Subcommand)]
enum KeysCommandArgs {
    List(KeysListArgs),
    Stats(KeysStatsArgs),
    ReplacementPlan(KeysReplacementPlanArgs),
    Import(KeysImportArgs),
    Probe(KeysProbeArgs),
    Disable(KeysDisableArgs),
    Restore(KeysRestoreArgs),
    ProbeApply {
        #[command(subcommand)]
        command: KeysProbeApplyCommandArgs,
    },
}

#[derive(Debug, Subcommand)]
enum KeysProbeApplyCommandArgs {
    Plan(KeysProbeApplyPlanArgs),
    Apply(KeysProbeApplyApplyArgs),
}

#[derive(Debug, Subcommand)]
enum FailuresCommand {
    Tail(FailuresTailArgs),
    Explain(FailuresExplainArgs),
}

#[derive(Debug, Args)]
#[command(
    long_about = "Create a local config template. Side-effect class: local_write unless --dry-run. Writes local files: yes unless --dry-run. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct InitLocalArgs {
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    keys: PathBuf,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long)]
    force: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Validate local configuration offline. Side-effect class: offline_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct CheckConfigArgs {
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Explain current runtime routing for one public model. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct RouteExplainArgs {
    model: String,
    #[arg(long = "client-token-ref")]
    client_token_ref: Option<String>,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "List compiled runtime public models. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct ModelsListArgs {
    #[arg(long = "client-token-ref")]
    client_token_ref: Option<String>,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Explain one compiled runtime public model. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct ModelsExplainArgs {
    #[arg(long)]
    model: String,
    #[arg(long = "client-token-ref")]
    client_token_ref: Option<String>,
    #[arg(long = "endpoint-family")]
    endpoint_family: Option<String>,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
    #[arg(long = "json", conflicts_with = "output")]
    json: bool,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Plan local model-route onboarding from read-only runtime projections. Side-effect class: runtime_readonly unless confirmed --apply. Writes local files: no. Calls upstreams: no. Confirmed --apply writes only the staged registry and does not reload active runtime."
)]
struct ModelsOnboardPlanArgs {
    #[arg(long)]
    channel: String,
    #[arg(long = "public-model")]
    public_model: String,
    #[arg(long = "upstream-model")]
    upstream_model: Option<String>,
    #[arg(long = "client-token-ref")]
    client_token_ref: Option<String>,
    #[arg(long = "endpoint-family")]
    endpoint_family: Option<String>,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    discover: bool,
    #[arg(long = "sync-plan")]
    sync_plan: bool,
    #[arg(long = "sync-apply")]
    sync_apply: bool,
    #[arg(long)]
    apply: bool,
    #[arg(long, requires = "apply")]
    expected_staged_registry_version: Option<u64>,
    #[arg(long)]
    reload: bool,
    #[arg(long = "reload-apply")]
    reload_apply: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Summarize runtime reload state from read-only management projections. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct ReloadStatusArgs {
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Report runtime reload diff availability from a read-only management projection. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct ReloadDiffArgs {
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Apply a planned runtime reload with an expected staged registry version precondition. Side-effect class: runtime_readonly with --dry-run, management_write with --yes. Writes local files: no. Calls upstreams: no. Confirmed apply can mutate active runtime."
)]
struct ReloadApplyArgs {
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long)]
    expected_staged_registry_version: Option<u64>,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "List runtime client-token references without raw token material. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct ClientTokensListArgs {
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "List credential-set summaries from read-only management projections. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct KeysListArgs {
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Show credential-set stats from bounded read-only management projections. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct KeysStatsArgs {
    #[arg(long = "credential-set")]
    credential_set_id: Option<String>,
    #[arg(long, requires = "credential_set_id")]
    include_credential_refs: bool,
    #[arg(
        long,
        default_value_t = 20,
        requires = "include_credential_refs",
        value_parser = parse_credential_ref_limit
    )]
    credential_ref_limit: usize,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Plan safe credential replacement from read-only management projections. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct KeysReplacementPlanArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long)]
    model: Option<String>,
    #[arg(long = "client-token-ref")]
    client_token_ref: Option<String>,
    #[arg(long, requires = "credential_set_id")]
    include_credential_refs: bool,
    #[arg(
        long,
        default_value_t = 20,
        requires = "include_credential_refs",
        value_parser = parse_credential_ref_limit
    )]
    credential_ref_limit: usize,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Import replacement credentials into a credential set. Side-effect class: local_preview with --dry-run, management_write with --yes. Reads local files: yes. Calls upstreams: no. Confirmed import writes management credential store and can mutate active runtime."
)]
struct KeysImportArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long)]
    source: PathBuf,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Probe exactly one credential by non-secret credential_ref. Side-effect class: runtime_readonly with --dry-run, upstream_touching with --yes. Calls upstreams: yes only when confirmed. Confirmed probe persists redacted probe evidence."
)]
struct KeysProbeArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long = "credential-ref")]
    credential_ref: String,
    #[arg(long)]
    model: String,
    #[arg(long, value_enum, default_value_t = crate::cli_commands::keys::KeysProbeKind::ModelRetrieve)]
    kind: crate::cli_commands::keys::KeysProbeKind,
    #[arg(long = "expected-output")]
    expected_output: Option<String>,
    #[arg(id = "probe_timeout_seconds", long = "probe-timeout-seconds")]
    timeout_seconds: Option<u64>,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Disable exactly one credential by non-secret credential_ref. Side-effect class: runtime_readonly with --dry-run, management_write with --yes. Calls upstreams: no. Confirmed disable mutates credential lifecycle state through management."
)]
struct KeysDisableArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long = "credential-ref")]
    credential_ref: String,
    #[arg(long)]
    reason: String,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Restore exactly one credential by non-secret credential_ref. Side-effect class: offline_readonly with --dry-run, management_write with --yes. Calls upstreams: no. Confirmed restore mutates credential lifecycle state through management."
)]
struct KeysRestoreArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long = "credential-ref")]
    credential_ref: String,
    #[arg(long)]
    reason: String,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Project the lifecycle action that the latest credential probe would apply. Side-effect class: runtime_readonly. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct KeysProbeApplyPlanArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long = "credential-ref")]
    credential_ref: String,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Apply the latest credential probe evidence explicitly. Side-effect class: runtime_readonly with --dry-run, management_write with --yes. Calls upstreams: no. Dry-run only reads the apply plan."
)]
struct KeysProbeApplyApplyArgs {
    #[arg(long = "credential-set")]
    credential_set_id: String,
    #[arg(long = "credential-ref")]
    credential_ref: String,
    #[arg(long = "probe-result-ref", required_unless_present = "dry_run")]
    probe_result_ref: Option<String>,
    #[arg(long, conflicts_with = "yes")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Tail bounded recent failure evidence. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct FailuresTailArgs {
    #[arg(long, value_parser = crate::cli_commands::failures::parse_failure_last)]
    last: Option<usize>,
    #[arg(long = "request-id")]
    request_id: Option<String>,
    #[arg(long = "model")]
    public_model: Option<String>,
    #[arg(long = "channel")]
    channel_id: Option<String>,
    #[arg(long)]
    directive: Option<String>,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    long_about = "Explain one request from bounded recent failure evidence. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct FailuresExplainArgs {
    request_id: String,
    #[arg(long, value_parser = crate::cli_commands::failures::parse_failure_last)]
    last: Option<usize>,
    #[arg(long = "model")]
    public_model: Option<String>,
    #[arg(long = "channel")]
    channel_id: Option<String>,
    #[arg(long)]
    directive: Option<String>,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Args)]
#[command(
    about = "Read-only runtime doctor",
    long_about = "Read-only runtime doctor. Side-effect class: runtime_readonly. Writes local files: no. Calls upstreams: no. Mutates management state or active runtime: no."
)]
struct DoctorArgs {
    #[arg(long)]
    include_alerts: bool,
    #[arg(long)]
    include_events: bool,
    #[arg(long)]
    include_routes: bool,
    #[arg(long, default_value_t = 20, value_parser = parse_doctor_event_limit)]
    event_limit: usize,
    #[arg(long, value_enum, default_value_t = crate::cli_report::OutputFormat::Table)]
    output: crate::cli_report::OutputFormat,
}

fn parse_credential_ref_limit(value: &str) -> Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| "credential-ref-limit must be an integer from 1 to 50".to_string())?;
    if (1..=50).contains(&limit) {
        Ok(limit)
    } else {
        Err("credential-ref-limit must be from 1 to 50".to_string())
    }
}

fn parse_doctor_event_limit(value: &str) -> Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| "event-limit must be an integer from 1 to 50".to_string())?;
    if (1..=50).contains(&limit) {
        Ok(limit)
    } else {
        Err("event-limit must be from 1 to 50".to_string())
    }
}

pub fn parse_action() -> Result<CliAction, clap::Error> {
    parse_action_from(std::env::args_os())
}

pub fn parse_action_from<I, T>(args: I) -> Result<CliAction, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    let operator_connection_options = cli.operator_connection_options();
    Ok(match cli.command {
        Some(CliCommand::Serve) | None => CliAction::Serve {
            config_path: cli.config,
        },
        Some(CliCommand::CheckConfig(args)) => CliAction::CheckConfig {
            config_path: cli.config.into(),
            output: args.output,
        },
        Some(CliCommand::Init {
            command: InitCommand::Local(args),
        }) => CliAction::InitLocal(crate::operator_templates::InitLocalOptions {
            out: args.out,
            keys: args.keys,
            mode: if args.dry_run {
                crate::operator_templates::InitLocalMode::DryRun
            } else if args.yes {
                crate::operator_templates::InitLocalMode::Write
            } else {
                crate::operator_templates::InitLocalMode::NeedsConfirmation
            },
            force: args.force,
            output: args.output,
        }),
        Some(CliCommand::Route {
            command: RouteCommand::Explain(args),
        }) => CliAction::RouteExplain(crate::cli_commands::route::RouteExplainOptions {
            connection: operator_connection_options,
            model: args.model,
            client_token_ref: args.client_token_ref,
            output: args.output,
        }),
        Some(CliCommand::Models {
            command: ModelsCommand::List(args),
        }) => CliAction::ModelsList(crate::cli_commands::models::ModelsListOptions {
            connection: operator_connection_options,
            client_token_ref: args.client_token_ref,
            output: args.output,
        }),
        Some(CliCommand::Models {
            command: ModelsCommand::Explain(args),
        }) => CliAction::ModelsExplain(crate::cli_commands::models::ModelsExplainOptions {
            connection: operator_connection_options,
            model: args.model,
            client_token_ref: args.client_token_ref,
            endpoint_family: args.endpoint_family,
            output: if args.json {
                crate::cli_report::OutputFormat::Json
            } else {
                args.output
            },
        }),
        Some(CliCommand::Models {
            command: ModelsCommand::OnboardPlan(args),
        }) => CliAction::ModelsOnboardPlan(
            crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                connection: operator_connection_options,
                channel_id: args.channel,
                public_model: args.public_model,
                upstream_model: args.upstream_model,
                client_token_ref: args.client_token_ref,
                endpoint_family: args.endpoint_family,
                mode: if args.discover
                    || args.sync_plan
                    || args.sync_apply
                    || args.reload
                    || args.reload_apply
                {
                    crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DeferredToSeparatePlan
                } else if args.apply && args.dry_run {
                    crate::cli_commands::models_onboard::ModelsOnboardPlanMode::ApplyDryRun
                } else if args.apply && args.yes {
                    crate::cli_commands::models_onboard::ModelsOnboardPlanMode::Apply
                } else if args.apply {
                    crate::cli_commands::models_onboard::ModelsOnboardPlanMode::NeedsConfirmation
                } else if args.dry_run && !args.yes {
                    crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DryRun
                } else {
                    crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DeferredToSeparatePlan
                },
                expected_staged_registry_version: args.expected_staged_registry_version,
                output: args.output,
            },
        ),
        Some(CliCommand::Reload {
            command: ReloadCommand::Status(args),
        }) => CliAction::ReloadStatus(crate::cli_commands::reload::ReloadStatusOptions {
            connection: operator_connection_options,
            output: args.output,
        }),
        Some(CliCommand::Reload {
            command: ReloadCommand::Diff(args),
        }) => CliAction::ReloadDiff(crate::cli_commands::reload::ReloadDiffOptions {
            connection: operator_connection_options,
            output: args.output,
        }),
        Some(CliCommand::Reload {
            command: ReloadCommand::Apply(args),
        }) => CliAction::ReloadApply(crate::cli_commands::reload::ReloadApplyOptions {
            connection: operator_connection_options,
            mode: if args.dry_run {
                crate::cli_commands::reload::ReloadApplyMode::DryRun
            } else if args.yes {
                crate::cli_commands::reload::ReloadApplyMode::Apply
            } else {
                crate::cli_commands::reload::ReloadApplyMode::NeedsConfirmation
            },
            expected_staged_registry_version: args.expected_staged_registry_version,
            output: args.output,
        }),
        Some(CliCommand::ClientTokens {
            command: ClientTokensCommand::List(args),
        }) => CliAction::ClientTokensList(
            crate::cli_commands::client_tokens::ClientTokensListOptions {
                connection: operator_connection_options,
                output: args.output,
            },
        ),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::List(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::List(
            crate::cli_commands::keys::KeysListOptions {
                connection: operator_connection_options,
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::Stats(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::Stats(
            crate::cli_commands::keys::KeysStatsOptions {
                connection: operator_connection_options,
                credential_set_id: args.credential_set_id,
                include_credential_refs: args.include_credential_refs,
                credential_ref_limit: args.credential_ref_limit,
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::ReplacementPlan(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::ReplacementPlan(
            crate::cli_commands::keys::KeysReplacementPlanOptions {
                connection: operator_connection_options,
                credential_set_id: args.credential_set_id,
                model: args.model,
                client_token_ref: args.client_token_ref,
                include_credential_refs: args.include_credential_refs,
                credential_ref_limit: args.credential_ref_limit,
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::Import(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(
            crate::cli_commands::keys::KeysImportOptions {
                connection: operator_connection_options,
                credential_set_id: args.credential_set_id,
                source: args.source,
                mode: if args.dry_run {
                    crate::cli_commands::keys::KeysImportMode::DryRun
                } else if args.yes {
                    crate::cli_commands::keys::KeysImportMode::Apply
                } else {
                    crate::cli_commands::keys::KeysImportMode::NeedsConfirmation
                },
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::Probe(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(
            crate::cli_commands::keys::KeysProbeOptions {
                connection: operator_connection_options,
                credential_set_id: args.credential_set_id,
                credential_ref: args.credential_ref,
                model: args.model,
                kind: args.kind,
                expected_output: args.expected_output,
                timeout_seconds: args.timeout_seconds,
                mode: if args.dry_run {
                    crate::cli_commands::keys::KeysProbeMode::DryRun
                } else if args.yes {
                    crate::cli_commands::keys::KeysProbeMode::Apply
                } else {
                    crate::cli_commands::keys::KeysProbeMode::NeedsConfirmation
                },
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::Disable(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::Disable(
            crate::cli_commands::keys::KeysDisableOptions {
                connection: operator_connection_options,
                credential_set_id: args.credential_set_id,
                credential_ref: args.credential_ref,
                reason: args.reason,
                mode: if args.dry_run {
                    crate::cli_commands::keys::KeysDisableMode::DryRun
                } else if args.yes {
                    crate::cli_commands::keys::KeysDisableMode::Apply
                } else {
                    crate::cli_commands::keys::KeysDisableMode::NeedsConfirmation
                },
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command: KeysCommandArgs::Restore(args),
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::Restore(
            crate::cli_commands::keys::KeysRestoreOptions {
                connection: operator_connection_options,
                credential_set_id: args.credential_set_id,
                credential_ref: args.credential_ref,
                reason: args.reason,
                mode: if args.dry_run {
                    crate::cli_commands::keys::KeysRestoreMode::DryRun
                } else if args.yes {
                    crate::cli_commands::keys::KeysRestoreMode::Apply
                } else {
                    crate::cli_commands::keys::KeysRestoreMode::NeedsConfirmation
                },
                output: args.output,
            },
        )),
        Some(CliCommand::Keys {
            command:
                KeysCommandArgs::ProbeApply {
                    command: KeysProbeApplyCommandArgs::Plan(args),
                },
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
            crate::cli_commands::keys::KeysProbeApplyCommand::Plan(
                crate::cli_commands::keys::KeysProbeApplyPlanOptions {
                    connection: operator_connection_options,
                    credential_set_id: args.credential_set_id,
                    credential_ref: args.credential_ref,
                    output: args.output,
                },
            ),
        )),
        Some(CliCommand::Keys {
            command:
                KeysCommandArgs::ProbeApply {
                    command: KeysProbeApplyCommandArgs::Apply(args),
                },
        }) => CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
            crate::cli_commands::keys::KeysProbeApplyCommand::Apply(
                crate::cli_commands::keys::KeysProbeApplyApplyOptions {
                    connection: operator_connection_options,
                    credential_set_id: args.credential_set_id,
                    credential_ref: args.credential_ref,
                    probe_result_ref: args.probe_result_ref,
                    mode: if args.dry_run {
                        crate::cli_commands::keys::KeysProbeApplyMode::DryRun
                    } else if args.yes {
                        crate::cli_commands::keys::KeysProbeApplyMode::Apply
                    } else {
                        crate::cli_commands::keys::KeysProbeApplyMode::NeedsConfirmation
                    },
                    output: args.output,
                },
            ),
        )),
        Some(CliCommand::Failures {
            command: FailuresCommand::Tail(args),
        }) => CliAction::FailuresTail(crate::cli_commands::failures::FailureTailOptions {
            connection: operator_connection_options,
            last: args.last,
            filters: crate::cli_commands::failures::FailureFilters {
                request_id: args.request_id,
                public_model: args.public_model,
                channel_id: args.channel_id,
                directive: args.directive,
            },
            output: args.output,
        }),
        Some(CliCommand::Failures {
            command: FailuresCommand::Explain(args),
        }) => CliAction::FailuresExplain(crate::cli_commands::failures::FailureExplainOptions {
            connection: operator_connection_options,
            request_id: args.request_id,
            last: args.last,
            filters: crate::cli_commands::failures::FailureFilters {
                request_id: None,
                public_model: args.public_model,
                channel_id: args.channel_id,
                directive: args.directive,
            },
            output: args.output,
        }),
        Some(CliCommand::Doctor(args)) => {
            CliAction::Doctor(crate::cli_commands::doctor::DoctorOptions {
                connection: operator_connection_options,
                include_alerts: args.include_alerts,
                include_events: args.include_events,
                include_routes: args.include_routes,
                event_limit: args.event_limit,
                output: args.output,
            })
        }
    })
}

#[cfg(test)]
pub fn parse_operator_connection_options_from<I, T>(
    args: I,
) -> Result<OperatorConnectionOptions, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    Ok(cli.operator_connection_options())
}

impl Cli {
    fn operator_connection_options(&self) -> OperatorConnectionOptions {
        OperatorConnectionOptions {
            management_url: self.management_url.clone(),
            deprecated_base_url: self.deprecated_base_url.clone(),
            management_token_env: self.management_token_env.clone(),
            management_token_stdin: self.management_token_stdin,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

#[cfg(test)]
pub fn render_help() -> String {
    Cli::command().render_help().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_local_dry_run_parse_selects_template_preview() {
        let action = parse_action_from([
            "one-ai-key",
            "init",
            "local",
            "--out",
            "config/local.yaml",
            "--keys",
            "data/relay.keys",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("init local dry-run should parse");

        assert_eq!(
            action,
            CliAction::InitLocal(crate::operator_templates::InitLocalOptions {
                out: "config/local.yaml".into(),
                keys: "data/relay.keys".into(),
                mode: crate::operator_templates::InitLocalMode::DryRun,
                force: false,
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn init_local_without_execution_flag_requires_confirmation() {
        let action = parse_action_from([
            "one-ai-key",
            "init",
            "local",
            "--out",
            "config/local.yaml",
            "--keys",
            "data/relay.keys",
        ])
        .expect("init local should parse before execution-mode validation");

        assert_eq!(
            action,
            CliAction::InitLocal(crate::operator_templates::InitLocalOptions {
                out: "config/local.yaml".into(),
                keys: "data/relay.keys".into(),
                mode: crate::operator_templates::InitLocalMode::NeedsConfirmation,
                force: false,
                output: crate::cli_report::OutputFormat::Table,
            })
        );
    }

    #[test]
    fn check_config_parse_uses_global_config_and_output_format() {
        let action = parse_action_from([
            "one-ai-key",
            "--config",
            "config/local.yaml",
            "check-config",
            "--output",
            "json",
        ])
        .expect("check-config should parse");

        assert_eq!(
            action,
            CliAction::CheckConfig {
                config_path: "config/local.yaml".into(),
                output: crate::cli_report::OutputFormat::Json,
            }
        );
    }

    #[test]
    fn management_url_flag_is_primary_and_base_url_is_deprecated_alias() {
        let options = parse_operator_connection_options_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--base-url",
            "https://deprecated.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "serve",
        ])
        .expect("operator connection flags should parse");

        assert_eq!(
            options.management_url.as_deref(),
            Some("https://router.example/v1")
        );
        assert_eq!(
            options.deprecated_base_url.as_deref(),
            Some("https://deprecated.example/v1")
        );
        assert_eq!(
            options.management_token_env.as_deref(),
            Some("ONE_AI_KEY_MANAGEMENT_TOKEN")
        );
        assert!(!options.management_token_stdin);
    }

    #[test]
    fn management_help_explains_management_url_and_token_boundary() {
        let help = render_help();

        assert!(help.contains("--management-url"));
        assert!(help.contains("/management/* endpoints"));
        assert!(help.contains("Client /v1 base URLs are rejected"));
        assert!(help.contains("management token"));
        assert!(help.contains("--base-url"));
        assert!(help.contains("Deprecated compatibility alias"));
    }

    #[test]
    fn root_help_parses_as_display_help_not_runtime_action() {
        let error =
            parse_action_from(["one-ai-key", "--help"]).expect_err("--help should render help");
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn doctor_help_names_runtime_readonly_boundary() {
        let mut command = Cli::command();
        let doctor = command
            .find_subcommand_mut("doctor")
            .expect("doctor subcommand should exist");
        let help = doctor.render_long_help().to_string();

        assert!(help.contains("Side-effect class: runtime_readonly"));
        assert!(help.contains("Writes local files: no"));
        assert!(help.contains("Calls upstreams: no"));
        assert!(help.contains("Mutates management state or active runtime: no"));
    }

    #[test]
    fn m2_command_help_names_side_effect_boundaries() {
        let commands = [
            (
                vec!["check-config"],
                "Side-effect class: offline_readonly",
                "Writes local files: no",
            ),
            (
                vec!["init", "local"],
                "Side-effect class: local_write unless --dry-run",
                "Calls upstreams: no",
            ),
            (
                vec!["route", "explain"],
                "Side-effect class: runtime_readonly",
                "Calls upstreams: no",
            ),
            (
                vec!["models", "list"],
                "Side-effect class: runtime_readonly",
                "Mutates management state or active runtime: no",
            ),
            (
                vec!["models", "explain"],
                "Side-effect class: runtime_readonly",
                "Mutates management state or active runtime: no",
            ),
            (
                vec!["client-tokens", "list"],
                "Side-effect class: runtime_readonly",
                "Writes local files: no",
            ),
            (
                vec!["keys", "list"],
                "Side-effect class: runtime_readonly",
                "Calls upstreams: no",
            ),
            (
                vec!["keys", "stats"],
                "Side-effect class: runtime_readonly",
                "Calls upstreams: no",
            ),
            (
                vec!["failures", "tail"],
                "Side-effect class: runtime_readonly",
                "Mutates management state or active runtime: no",
            ),
            (
                vec!["failures", "explain"],
                "Side-effect class: runtime_readonly",
                "Mutates management state or active runtime: no",
            ),
        ];

        for (path, side_effect, boundary) in commands {
            let mut command = Cli::command();
            let mut cursor = &mut command;
            for name in path {
                cursor = cursor
                    .find_subcommand_mut(name)
                    .unwrap_or_else(|| panic!("{name} subcommand should exist"));
            }
            let help = cursor.render_long_help().to_string();
            assert!(help.contains(side_effect), "{help}");
            assert!(help.contains(boundary), "{help}");
        }
    }

    #[test]
    fn route_explain_parse_uses_management_options_and_client_token_ref() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "route",
            "explain",
            "gpt-4o",
            "--client-token-ref",
            "local-client",
            "--output",
            "json",
        ])
        .expect("route explain should parse");

        assert_eq!(
            action,
            CliAction::RouteExplain(crate::cli_commands::route::RouteExplainOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example/v1".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                model: "gpt-4o".to_string(),
                client_token_ref: Some("local-client".to_string()),
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn models_list_parse_uses_management_options_and_client_token_ref() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "list",
            "--client-token-ref",
            "local-client",
            "--output",
            "json",
        ])
        .expect("models list should parse");

        assert_eq!(
            action,
            CliAction::ModelsList(crate::cli_commands::models::ModelsListOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example/v1".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                client_token_ref: Some("local-client".to_string()),
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn models_explain_parse_uses_explicit_model_flag() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "explain",
            "--model",
            "gpt-4o",
            "--output",
            "json",
        ])
        .expect("models explain should parse");

        assert_eq!(
            action,
            CliAction::ModelsExplain(crate::cli_commands::models::ModelsExplainOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example/v1".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                model: "gpt-4o".to_string(),
                client_token_ref: None,
                endpoint_family: None,
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn models_explain_parse_accepts_endpoint_family_and_client_token_ref() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "explain",
            "--model",
            "gpt-4o",
            "--endpoint-family",
            "chat_completions",
            "--client-token-ref",
            "local-client",
        ])
        .expect("models explain endpoint family should parse");

        assert_eq!(
            action,
            CliAction::ModelsExplain(crate::cli_commands::models::ModelsExplainOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example/v1".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                model: "gpt-4o".to_string(),
                client_token_ref: Some("local-client".to_string()),
                endpoint_family: Some("chat_completions".to_string()),
                output: crate::cli_report::OutputFormat::Table,
            })
        );
    }

    #[test]
    fn models_explain_parse_accepts_json_alias() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "explain",
            "--model",
            "gpt-4o",
            "--json",
        ])
        .expect("models explain --json alias should parse");

        assert_eq!(
            action,
            CliAction::ModelsExplain(crate::cli_commands::models::ModelsExplainOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example/v1".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                model: "gpt-4o".to_string(),
                client_token_ref: None,
                endpoint_family: None,
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn models_onboard_plan_parse_requires_explicit_dry_run_request() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "onboard-plan",
            "--channel",
            "relay-a",
            "--public-model",
            "coding",
            "--upstream-model",
            "vendor/coding",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("models onboard-plan dry-run should parse");

        assert_eq!(
            action,
            CliAction::ModelsOnboardPlan(
                crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    channel_id: "relay-a".to_string(),
                    public_model: "coding".to_string(),
                    upstream_model: Some("vendor/coding".to_string()),
                    client_token_ref: None,
                    endpoint_family: None,
                    mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DryRun,
                    expected_staged_registry_version: None,
                    output: crate::cli_report::OutputFormat::Json,
                }
            )
        );
    }

    #[test]
    fn models_onboard_plan_parse_accepts_visibility_projection_args() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "onboard-plan",
            "--channel",
            "relay-a",
            "--public-model",
            "coding",
            "--upstream-model",
            "vendor/coding",
            "--client-token-ref",
            "operator-client",
            "--endpoint-family",
            "chat_completions",
            "--dry-run",
        ])
        .expect("models onboard-plan visibility args should parse");

        assert_eq!(
            action,
            CliAction::ModelsOnboardPlan(
                crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    channel_id: "relay-a".to_string(),
                    public_model: "coding".to_string(),
                    upstream_model: Some("vendor/coding".to_string()),
                    client_token_ref: Some("operator-client".to_string()),
                    endpoint_family: Some("chat_completions".to_string()),
                    mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DryRun,
                    expected_staged_registry_version: None,
                    output: crate::cli_report::OutputFormat::Table,
                }
            )
        );
    }

    #[test]
    fn models_onboard_plan_parse_accepts_apply_precondition_and_yes() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "models",
            "onboard-plan",
            "--channel",
            "relay-a",
            "--public-model",
            "coding",
            "--upstream-model",
            "vendor/coding",
            "--apply",
            "--expected-staged-registry-version",
            "12",
            "--yes",
            "--output",
            "json",
        ])
        .expect("models onboard-plan confirmed apply should parse");

        assert_eq!(
            action,
            CliAction::ModelsOnboardPlan(
                crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    channel_id: "relay-a".to_string(),
                    public_model: "coding".to_string(),
                    upstream_model: Some("vendor/coding".to_string()),
                    client_token_ref: None,
                    endpoint_family: None,
                    mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::Apply,
                    expected_staged_registry_version: Some(12),
                    output: crate::cli_report::OutputFormat::Json,
                }
            )
        );
    }

    #[test]
    fn models_onboard_plan_parse_apply_without_yes_needs_confirmation() {
        let action = parse_action_from([
            "one-ai-key",
            "models",
            "onboard-plan",
            "--channel",
            "relay-a",
            "--public-model",
            "coding",
            "--apply",
            "--expected-staged-registry-version",
            "12",
        ])
        .expect("models onboard-plan apply without yes should parse before confirmation policy");

        assert!(matches!(
            action,
            CliAction::ModelsOnboardPlan(
                crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                    mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::NeedsConfirmation,
                    expected_staged_registry_version: Some(12),
                    ..
                }
            )
        ));
    }

    #[test]
    fn models_onboard_plan_parse_apply_dry_run_is_readonly_apply_preview() {
        let action = parse_action_from([
            "one-ai-key",
            "models",
            "onboard-plan",
            "--channel",
            "relay-a",
            "--public-model",
            "coding",
            "--upstream-model",
            "vendor/coding",
            "--apply",
            "--dry-run",
        ])
        .expect("models onboard-plan apply dry-run should parse");

        assert!(matches!(
            action,
            CliAction::ModelsOnboardPlan(
                crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                    mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::ApplyDryRun,
                    expected_staged_registry_version: None,
                    ..
                }
            )
        ));
    }

    #[test]
    fn models_onboard_plan_live_variants_parse_as_deferred() {
        for flag in [
            "--discover",
            "--sync-plan",
            "--sync-apply",
            "--reload",
            "--reload-apply",
            "--yes",
        ] {
            let action = parse_action_from([
                "one-ai-key",
                "models",
                "onboard-plan",
                "--channel",
                "relay-a",
                "--public-model",
                "coding",
                flag,
            ])
            .expect("deferred live variant should parse before policy denial");

            assert!(matches!(
                action,
                CliAction::ModelsOnboardPlan(
                    crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                        mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DeferredToSeparatePlan,
                        ..
                    }
                )
            ));
        }
    }

    #[test]
    fn models_onboard_plan_help_names_planning_only_boundary() {
        let mut command = Cli::command();
        let models = command
            .find_subcommand_mut("models")
            .expect("models subcommand should exist");
        let onboard = models
            .find_subcommand_mut("onboard-plan")
            .expect("models onboard-plan subcommand should exist");
        let help = onboard.render_long_help().to_string();

        assert!(help.contains("Side-effect class: runtime_readonly"));
        assert!(help.contains("Writes local files: no"));
        assert!(help.contains("Calls upstreams: no"));
        assert!(help.contains("Confirmed --apply writes only the staged registry"));
    }

    #[test]
    fn reload_status_cli_parse_uses_management_options() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "reload",
            "status",
            "--output",
            "json",
        ])
        .expect("reload status should parse");

        assert_eq!(
            action,
            CliAction::ReloadStatus(crate::cli_commands::reload::ReloadStatusOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn reload_apply_cli_parse_uses_dry_run_or_confirmed_precondition() {
        let dry_run = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "reload",
            "apply",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("reload apply --dry-run should parse");

        assert_eq!(
            dry_run,
            CliAction::ReloadApply(crate::cli_commands::reload::ReloadApplyOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                mode: crate::cli_commands::reload::ReloadApplyMode::DryRun,
                expected_staged_registry_version: None,
                output: crate::cli_report::OutputFormat::Json,
            })
        );

        let confirmed = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "reload",
            "apply",
            "--expected-staged-registry-version",
            "4",
            "--yes",
            "--output",
            "json",
        ])
        .expect("reload apply --yes should parse");

        assert_eq!(
            confirmed,
            CliAction::ReloadApply(crate::cli_commands::reload::ReloadApplyOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                mode: crate::cli_commands::reload::ReloadApplyMode::Apply,
                expected_staged_registry_version: Some(4),
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn client_tokens_list_parse_uses_management_options() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "client-tokens",
            "list",
            "--output",
            "json",
        ])
        .expect("client-tokens list should parse");

        assert_eq!(
            action,
            CliAction::ClientTokensList(
                crate::cli_commands::client_tokens::ClientTokensListOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example/v1".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    output: crate::cli_report::OutputFormat::Json,
                }
            )
        );
    }

    #[test]
    fn keys_list_parse_uses_management_options() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "list",
            "--output",
            "json",
        ])
        .expect("keys list should parse");

        assert_eq!(
            action,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::List(
                crate::cli_commands::keys::KeysListOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example/v1".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    output: crate::cli_report::OutputFormat::Json,
                },
            ))
        );
    }

    #[test]
    fn keys_stats_parse_supports_bounded_credential_refs_for_single_set() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "stats",
            "--credential-set",
            "relay-a",
            "--include-credential-refs",
            "--credential-ref-limit",
            "7",
            "--output",
            "json",
        ])
        .expect("keys stats should parse");

        assert_eq!(
            action,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Stats(
                crate::cli_commands::keys::KeysStatsOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example/v1".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    credential_set_id: Some("relay-a".to_string()),
                    include_credential_refs: true,
                    credential_ref_limit: 7,
                    output: crate::cli_report::OutputFormat::Json,
                },
            ))
        );
    }

    #[test]
    fn keys_replacement_plan_parse_supports_route_context_and_bounded_refs() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "replacement-plan",
            "--credential-set",
            "relay-credentials",
            "--model",
            "gpt-example",
            "--client-token-ref",
            "local-client",
            "--include-credential-refs",
            "--credential-ref-limit",
            "7",
            "--output",
            "json",
        ])
        .expect("keys replacement-plan should parse");

        assert_eq!(
            action,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::ReplacementPlan(
                crate::cli_commands::keys::KeysReplacementPlanOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    credential_set_id: "relay-credentials".to_string(),
                    model: Some("gpt-example".to_string()),
                    client_token_ref: Some("local-client".to_string()),
                    include_credential_refs: true,
                    credential_ref_limit: 7,
                    output: crate::cli_report::OutputFormat::Json,
                }
            ))
        );
    }

    #[test]
    fn keys_import_parse_supports_dry_run_and_confirmation_modes() {
        let dry_run = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "import",
            "--credential-set",
            "relay-credentials",
            "--source",
            "data/replacement.keys",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("keys import dry-run should parse");

        assert_eq!(
            dry_run,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(
                crate::cli_commands::keys::KeysImportOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    credential_set_id: "relay-credentials".to_string(),
                    source: "data/replacement.keys".into(),
                    mode: crate::cli_commands::keys::KeysImportMode::DryRun,
                    output: crate::cli_report::OutputFormat::Json,
                }
            ))
        );

        let needs_confirmation = parse_action_from([
            "one-ai-key",
            "keys",
            "import",
            "--credential-set",
            "relay-credentials",
            "--source",
            "data/replacement.keys",
        ])
        .expect("keys import should parse before confirmation validation");

        assert!(matches!(
            needs_confirmation,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(
                crate::cli_commands::keys::KeysImportOptions {
                    mode: crate::cli_commands::keys::KeysImportMode::NeedsConfirmation,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn keys_probe_parse_supports_dry_run_and_confirmation_modes() {
        let dry_run = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "probe",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--model",
            "gpt-example",
            "--kind",
            "chat-completion",
            "--expected-output",
            "ok",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("keys probe dry-run should parse");

        assert_eq!(
            dry_run,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(
                crate::cli_commands::keys::KeysProbeOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    credential_set_id: "relay-credentials".to_string(),
                    credential_ref: "cr:v1:pos:0".to_string(),
                    model: "gpt-example".to_string(),
                    kind: crate::cli_commands::keys::KeysProbeKind::ChatCompletion,
                    expected_output: Some("ok".to_string()),
                    timeout_seconds: None,
                    mode: crate::cli_commands::keys::KeysProbeMode::DryRun,
                    output: crate::cli_report::OutputFormat::Json,
                }
            ))
        );

        let needs_confirmation = parse_action_from([
            "one-ai-key",
            "keys",
            "probe",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--model",
            "gpt-example",
        ])
        .expect("keys probe should parse before confirmation validation");

        assert!(matches!(
            needs_confirmation,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(
                crate::cli_commands::keys::KeysProbeOptions {
                    mode: crate::cli_commands::keys::KeysProbeMode::NeedsConfirmation,
                    kind: crate::cli_commands::keys::KeysProbeKind::ModelRetrieve,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn keys_disable_parse_supports_dry_run_and_confirmation_modes() {
        let dry_run = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "disable",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified bad key",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("keys disable dry-run should parse");

        assert_eq!(
            dry_run,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Disable(
                crate::cli_commands::keys::KeysDisableOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    credential_set_id: "relay-credentials".to_string(),
                    credential_ref: "cr:v1:pos:0".to_string(),
                    reason: "operator verified bad key".to_string(),
                    mode: crate::cli_commands::keys::KeysDisableMode::DryRun,
                    output: crate::cli_report::OutputFormat::Json,
                }
            ))
        );

        let needs_confirmation = parse_action_from([
            "one-ai-key",
            "keys",
            "disable",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified bad key",
        ])
        .expect("keys disable should parse before confirmation validation");

        assert!(matches!(
            needs_confirmation,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Disable(
                crate::cli_commands::keys::KeysDisableOptions {
                    mode: crate::cli_commands::keys::KeysDisableMode::NeedsConfirmation,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn keys_disable_rejects_dry_run_yes_combination() {
        let parsed = parse_action_from([
            "one-ai-key",
            "keys",
            "disable",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified bad key",
            "--dry-run",
            "--yes",
        ]);

        assert!(parsed.is_err());
    }

    #[test]
    fn keys_restore_parse_supports_dry_run_confirmation_and_apply_modes() {
        let dry_run = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "restore",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified recovered credential",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("keys restore dry-run should parse");

        assert_eq!(
            dry_run,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Restore(
                crate::cli_commands::keys::KeysRestoreOptions {
                    connection: OperatorConnectionOptions {
                        management_url: Some("https://router.example".to_string()),
                        deprecated_base_url: None,
                        management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                        management_token_stdin: false,
                        timeout_seconds: 10,
                    },
                    credential_set_id: "relay-credentials".to_string(),
                    credential_ref: "cr:v1:pos:0".to_string(),
                    reason: "operator verified recovered credential".to_string(),
                    mode: crate::cli_commands::keys::KeysRestoreMode::DryRun,
                    output: crate::cli_report::OutputFormat::Json,
                }
            ))
        );

        let needs_confirmation = parse_action_from([
            "one-ai-key",
            "keys",
            "restore",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified recovered credential",
        ])
        .expect("keys restore should parse before confirmation validation");

        assert!(matches!(
            needs_confirmation,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Restore(
                crate::cli_commands::keys::KeysRestoreOptions {
                    mode: crate::cli_commands::keys::KeysRestoreMode::NeedsConfirmation,
                    ..
                }
            ))
        ));

        let apply = parse_action_from([
            "one-ai-key",
            "keys",
            "restore",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified recovered credential",
            "--yes",
        ])
        .expect("keys restore apply should parse");

        assert!(matches!(
            apply,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Restore(
                crate::cli_commands::keys::KeysRestoreOptions {
                    mode: crate::cli_commands::keys::KeysRestoreMode::Apply,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn keys_restore_rejects_dry_run_yes_combination() {
        let parsed = parse_action_from([
            "one-ai-key",
            "keys",
            "restore",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--reason",
            "operator verified recovered credential",
            "--dry-run",
            "--yes",
        ]);

        assert!(parsed.is_err());
    }

    #[test]
    fn keys_probe_apply_dry_run_parse_supports_plan_and_apply_dry_run() {
        let plan = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "probe-apply",
            "plan",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--output",
            "json",
        ])
        .expect("keys probe-apply plan should parse");

        assert_eq!(
            plan,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
                crate::cli_commands::keys::KeysProbeApplyCommand::Plan(
                    crate::cli_commands::keys::KeysProbeApplyPlanOptions {
                        connection: OperatorConnectionOptions {
                            management_url: Some("https://router.example".to_string()),
                            deprecated_base_url: None,
                            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                            management_token_stdin: false,
                            timeout_seconds: 10,
                        },
                        credential_set_id: "relay-credentials".to_string(),
                        credential_ref: "cr:v1:pos:0".to_string(),
                        output: crate::cli_report::OutputFormat::Json,
                    }
                )
            ))
        );

        let apply_dry_run = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "probe-apply",
            "apply",
            "--credential-set",
            "relay-credentials",
            "--credential-ref",
            "cr:v1:pos:0",
            "--probe-result-ref",
            "pr:v1:id:7",
            "--dry-run",
            "--output",
            "json",
        ])
        .expect("keys probe-apply apply --dry-run should parse");

        assert_eq!(
            apply_dry_run,
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
                crate::cli_commands::keys::KeysProbeApplyCommand::Apply(
                    crate::cli_commands::keys::KeysProbeApplyApplyOptions {
                        connection: OperatorConnectionOptions {
                            management_url: Some("https://router.example".to_string()),
                            deprecated_base_url: None,
                            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                            management_token_stdin: false,
                            timeout_seconds: 10,
                        },
                        credential_set_id: "relay-credentials".to_string(),
                        credential_ref: "cr:v1:pos:0".to_string(),
                        probe_result_ref: Some("pr:v1:id:7".to_string()),
                        mode: crate::cli_commands::keys::KeysProbeApplyMode::DryRun,
                        output: crate::cli_report::OutputFormat::Json,
                    }
                )
            ))
        );
    }

    #[test]
    fn keys_stats_rejects_out_of_range_credential_ref_limit() {
        let parsed = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example/v1",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "keys",
            "stats",
            "--credential-set",
            "relay-a",
            "--include-credential-refs",
            "--credential-ref-limit",
            "500",
        ]);

        assert!(parsed.is_err());
    }

    #[test]
    fn failures_tail_parse_uses_bounded_filters_and_management_options() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "failures",
            "tail",
            "--last",
            "20",
            "--request-id",
            "req_123",
            "--model",
            "gpt-example",
            "--channel",
            "relay-a",
            "--directive",
            "retry",
            "--output",
            "json",
        ])
        .expect("failures tail should parse");

        assert_eq!(
            action,
            CliAction::FailuresTail(crate::cli_commands::failures::FailureTailOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                last: Some(20),
                filters: crate::cli_commands::failures::FailureFilters {
                    request_id: Some("req_123".to_string()),
                    public_model: Some("gpt-example".to_string()),
                    channel_id: Some("relay-a".to_string()),
                    directive: Some("retry".to_string()),
                },
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn failures_explain_parse_uses_request_id_and_window_limit() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "failures",
            "explain",
            "req_123",
            "--last",
            "50",
            "--output",
            "json",
        ])
        .expect("failures explain should parse");

        assert_eq!(
            action,
            CliAction::FailuresExplain(crate::cli_commands::failures::FailureExplainOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                request_id: "req_123".to_string(),
                last: Some(50),
                filters: crate::cli_commands::failures::FailureFilters::default(),
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }

    #[test]
    fn doctor_parse_uses_management_options_and_bounded_opt_in_flags() {
        let action = parse_action_from([
            "one-ai-key",
            "--management-url",
            "https://router.example",
            "--management-token-env",
            "ONE_AI_KEY_MANAGEMENT_TOKEN",
            "doctor",
            "--include-alerts",
            "--include-events",
            "--include-routes",
            "--event-limit",
            "20",
            "--output",
            "json",
        ])
        .expect("doctor should parse");

        assert_eq!(
            action,
            CliAction::Doctor(crate::cli_commands::doctor::DoctorOptions {
                connection: OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                include_alerts: true,
                include_events: true,
                include_routes: true,
                event_limit: 20,
                output: crate::cli_report::OutputFormat::Json,
            })
        );
    }
}
