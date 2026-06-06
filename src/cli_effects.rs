use crate::{cli::CliAction, operator_templates::InitLocalMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffectClass {
    OfflineReadonly,
    RuntimeReadonly,
    LocalPreview,
    LocalWrite,
    ManagementWrite,
    UpstreamTouching,
    ServeProcess,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectVector {
    pub reads_local_files: bool,
    pub reads_management_runtime: bool,
    pub reads_management_store: bool,
    pub writes_local_files: bool,
    pub writes_management_store: bool,
    pub calls_upstream: bool,
    pub mutates_runtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandEffect {
    pub side_effect_class: SideEffectClass,
    pub effect_vector: EffectVector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationOutcome {
    Allowed,
    PromptRequired {
        reason_code: &'static str,
    },
    Denied {
        exit_code: i32,
        reason_code: &'static str,
    },
}

pub fn classify_action(action: &CliAction) -> CommandEffect {
    match action {
        CliAction::Serve { .. } => CommandEffect {
            side_effect_class: SideEffectClass::ServeProcess,
            effect_vector: EffectVector {
                reads_local_files: true,
                mutates_runtime: true,
                ..EffectVector::default()
            },
        },
        CliAction::InitLocal(options) => match options.mode {
            InitLocalMode::DryRun => CommandEffect {
                side_effect_class: SideEffectClass::OfflineReadonly,
                effect_vector: EffectVector::default(),
            },
            InitLocalMode::Write | InitLocalMode::NeedsConfirmation => CommandEffect {
                side_effect_class: SideEffectClass::LocalWrite,
                effect_vector: EffectVector {
                    writes_local_files: true,
                    ..EffectVector::default()
                },
            },
        },
        CliAction::CheckConfig { .. } => CommandEffect {
            side_effect_class: SideEffectClass::OfflineReadonly,
            effect_vector: EffectVector {
                reads_local_files: true,
                ..EffectVector::default()
            },
        },
        CliAction::RouteExplain(_)
        | CliAction::ModelsList(_)
        | CliAction::ModelsExplain(_)
        | CliAction::ModelsOnboardPlan(_)
        | CliAction::ReloadStatus(_)
        | CliAction::ClientTokensList(_) => CommandEffect {
            side_effect_class: SideEffectClass::RuntimeReadonly,
            effect_vector: EffectVector {
                reads_management_runtime: true,
                ..EffectVector::default()
            },
        },
        CliAction::ReloadDiff(_) => runtime_readonly_effect(),
        CliAction::ReloadApply(options) => match options.mode {
            crate::cli_commands::reload::ReloadApplyMode::DryRun => runtime_readonly_effect(),
            crate::cli_commands::reload::ReloadApplyMode::NeedsConfirmation
            | crate::cli_commands::reload::ReloadApplyMode::Apply => {
                crate::cli_commands::reload::reload_apply_management_effect()
            }
        },
        CliAction::Keys(command) => match command {
            crate::cli_commands::keys::KeysCommand::Import(options) => match options.mode {
                crate::cli_commands::keys::KeysImportMode::DryRun => CommandEffect {
                    side_effect_class: SideEffectClass::LocalPreview,
                    effect_vector: EffectVector {
                        reads_local_files: true,
                        ..EffectVector::default()
                    },
                },
                crate::cli_commands::keys::KeysImportMode::Apply
                | crate::cli_commands::keys::KeysImportMode::NeedsConfirmation => CommandEffect {
                    side_effect_class: SideEffectClass::ManagementWrite,
                    effect_vector: EffectVector {
                        reads_local_files: true,
                        writes_management_store: true,
                        mutates_runtime: true,
                        ..EffectVector::default()
                    },
                },
            },
            crate::cli_commands::keys::KeysCommand::Probe(options) => match options.mode {
                crate::cli_commands::keys::KeysProbeMode::DryRun => {
                    runtime_readonly_store_reads_effect()
                }
                crate::cli_commands::keys::KeysProbeMode::Apply
                | crate::cli_commands::keys::KeysProbeMode::NeedsConfirmation => CommandEffect {
                    side_effect_class: SideEffectClass::UpstreamTouching,
                    effect_vector: EffectVector {
                        reads_management_runtime: true,
                        writes_management_store: true,
                        calls_upstream: true,
                        ..EffectVector::default()
                    },
                },
            },
            crate::cli_commands::keys::KeysCommand::ProbeApply(command) => match command {
                crate::cli_commands::keys::KeysProbeApplyCommand::Plan(_) => {
                    runtime_readonly_store_reads_effect()
                }
                crate::cli_commands::keys::KeysProbeApplyCommand::Apply(options) => {
                    match options.mode {
                        crate::cli_commands::keys::KeysProbeApplyMode::DryRun => {
                            runtime_readonly_store_reads_effect()
                        }
                        crate::cli_commands::keys::KeysProbeApplyMode::Apply
                        | crate::cli_commands::keys::KeysProbeApplyMode::NeedsConfirmation => {
                            CommandEffect {
                                side_effect_class: SideEffectClass::ManagementWrite,
                                effect_vector: EffectVector {
                                    reads_management_runtime: true,
                                    reads_management_store: true,
                                    writes_management_store: true,
                                    mutates_runtime: true,
                                    ..EffectVector::default()
                                },
                            }
                        }
                    }
                }
            },
            crate::cli_commands::keys::KeysCommand::List(_)
            | crate::cli_commands::keys::KeysCommand::Stats(_) => {
                runtime_readonly_store_reads_effect()
            }
        },
        CliAction::FailuresTail(_) | CliAction::FailuresExplain(_) => {
            runtime_readonly_store_reads_effect()
        }
        CliAction::Doctor(options) => {
            let mut effect = runtime_readonly_effect();
            if options.include_alerts || options.include_events {
                effect.effect_vector.reads_management_store = true;
            }
            effect
        }
    }
}

pub fn runtime_readonly_effect() -> CommandEffect {
    CommandEffect {
        side_effect_class: SideEffectClass::RuntimeReadonly,
        effect_vector: EffectVector {
            reads_management_runtime: true,
            ..EffectVector::default()
        },
    }
}

pub fn runtime_readonly_store_reads_effect() -> CommandEffect {
    CommandEffect {
        side_effect_class: SideEffectClass::RuntimeReadonly,
        effect_vector: EffectVector {
            reads_management_runtime: true,
            reads_management_store: true,
            ..EffectVector::default()
        },
    }
}

pub fn side_effect_class_code(side_effect_class: SideEffectClass) -> &'static str {
    match side_effect_class {
        SideEffectClass::OfflineReadonly => "offline_readonly",
        SideEffectClass::RuntimeReadonly => "runtime_readonly",
        SideEffectClass::LocalPreview => "local_preview",
        SideEffectClass::LocalWrite => "local_write",
        SideEffectClass::ManagementWrite => "management_write",
        SideEffectClass::UpstreamTouching => "upstream_touching",
        SideEffectClass::ServeProcess => "serve_process",
    }
}

pub fn effect_vector_json(effect_vector: EffectVector) -> serde_json::Value {
    serde_json::json!({
        "reads_local_files": effect_vector.reads_local_files,
        "reads_management_runtime": effect_vector.reads_management_runtime,
        "reads_management_store": effect_vector.reads_management_store,
        "writes_local_files": effect_vector.writes_local_files,
        "writes_management_store": effect_vector.writes_management_store,
        "calls_upstream": effect_vector.calls_upstream,
        "mutates_runtime": effect_vector.mutates_runtime,
    })
}

pub fn confirmation_outcome(action: &CliAction, stdin_is_tty: bool) -> ConfirmationOutcome {
    match action {
        CliAction::InitLocal(options)
            if matches!(options.mode, InitLocalMode::NeedsConfirmation) =>
        {
            if stdin_is_tty {
                ConfirmationOutcome::PromptRequired {
                    reason_code: "confirmation_required",
                }
            } else {
                ConfirmationOutcome::Denied {
                    exit_code: 3,
                    reason_code: "confirmation_required",
                }
            }
        }
        CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(options))
            if matches!(
                options.mode,
                crate::cli_commands::keys::KeysImportMode::NeedsConfirmation
            ) =>
        {
            if stdin_is_tty {
                ConfirmationOutcome::PromptRequired {
                    reason_code: "confirmation_required",
                }
            } else {
                ConfirmationOutcome::Denied {
                    exit_code: 3,
                    reason_code: "confirmation_required",
                }
            }
        }
        CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(options))
            if matches!(
                options.mode,
                crate::cli_commands::keys::KeysProbeMode::NeedsConfirmation
            ) =>
        {
            if stdin_is_tty {
                ConfirmationOutcome::PromptRequired {
                    reason_code: "confirmation_required",
                }
            } else {
                ConfirmationOutcome::Denied {
                    exit_code: 3,
                    reason_code: "confirmation_required",
                }
            }
        }
        CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
            crate::cli_commands::keys::KeysProbeApplyCommand::Apply(options),
        )) if matches!(
            options.mode,
            crate::cli_commands::keys::KeysProbeApplyMode::NeedsConfirmation
        ) =>
        {
            if stdin_is_tty {
                ConfirmationOutcome::PromptRequired {
                    reason_code: "confirmation_required",
                }
            } else {
                ConfirmationOutcome::Denied {
                    exit_code: 3,
                    reason_code: "confirmation_required",
                }
            }
        }
        CliAction::ReloadApply(options)
            if matches!(
                options.mode,
                crate::cli_commands::reload::ReloadApplyMode::NeedsConfirmation
            ) =>
        {
            if stdin_is_tty {
                ConfirmationOutcome::PromptRequired {
                    reason_code: "confirmation_required",
                }
            } else {
                ConfirmationOutcome::Denied {
                    exit_code: 3,
                    reason_code: "confirmation_required",
                }
            }
        }
        CliAction::ModelsOnboardPlan(options)
            if matches!(
                options.mode,
                crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DeferredToSeparatePlan
            ) =>
        {
            ConfirmationOutcome::Denied {
                exit_code: 3,
                reason_code: "deferred_to_separate_plan",
            }
        }
        _ => ConfirmationOutcome::Allowed,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        cli::{parse_action_from, CliAction},
        operator_templates::{InitLocalMode, InitLocalOptions},
    };
    use std::path::PathBuf;

    #[test]
    fn classifies_check_config_as_offline_readonly() {
        let action = CliAction::CheckConfig {
            config_path: PathBuf::from("config/local.yaml"),
            output: crate::cli_report::OutputFormat::Table,
        };

        let effect = super::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            super::SideEffectClass::OfflineReadonly
        );
        assert_eq!(
            effect.effect_vector,
            super::EffectVector {
                reads_local_files: true,
                reads_management_runtime: false,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );
    }

    #[test]
    fn classifies_init_local_dry_run_as_offline_readonly() {
        let action = CliAction::InitLocal(InitLocalOptions {
            out: PathBuf::from("config/local.yaml"),
            keys: PathBuf::from("data/relay.keys"),
            mode: InitLocalMode::DryRun,
            force: false,
            output: crate::cli_report::OutputFormat::Table,
        });

        let effect = super::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            super::SideEffectClass::OfflineReadonly
        );
        assert_eq!(
            effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: false,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );
    }

    #[test]
    fn classifies_init_local_yes_as_local_write() {
        let action = CliAction::InitLocal(InitLocalOptions {
            out: PathBuf::from("config/local.yaml"),
            keys: PathBuf::from("data/relay.keys"),
            mode: InitLocalMode::Write,
            force: false,
            output: crate::cli_report::OutputFormat::Table,
        });

        let effect = super::classify_action(&action);

        assert_eq!(effect.side_effect_class, super::SideEffectClass::LocalWrite);
        assert!(effect.effect_vector.writes_local_files);
        assert!(!effect.effect_vector.calls_upstream);
        assert!(!effect.effect_vector.writes_management_store);
    }

    #[test]
    fn rejects_dry_run_yes_combination() {
        let parsed = parse_action_from([
            "one-ai-key",
            "init",
            "local",
            "--out",
            "config/local.yaml",
            "--keys",
            "data/relay.keys",
            "--dry-run",
            "--yes",
        ]);

        assert!(parsed.is_err());
    }

    #[test]
    fn non_tty_write_without_yes_requires_confirmation() {
        let action = CliAction::InitLocal(InitLocalOptions {
            out: PathBuf::from("config/local.yaml"),
            keys: PathBuf::from("data/relay.keys"),
            mode: InitLocalMode::NeedsConfirmation,
            force: false,
            output: crate::cli_report::OutputFormat::Table,
        });

        let outcome = super::confirmation_outcome(&action, false);

        assert_eq!(
            outcome,
            super::ConfirmationOutcome::Denied {
                exit_code: 3,
                reason_code: "confirmation_required"
            }
        );
    }

    #[test]
    fn keys_import_requires_confirmation_before_management_write() {
        let action = CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(
            crate::cli_commands::keys::KeysImportOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-a".to_string(),
                source: PathBuf::from("data/replacement.keys"),
                mode: crate::cli_commands::keys::KeysImportMode::NeedsConfirmation,
                output: crate::cli_report::OutputFormat::Table,
            },
        ));

        assert_eq!(
            super::confirmation_outcome(&action, false),
            super::ConfirmationOutcome::Denied {
                exit_code: 3,
                reason_code: "confirmation_required"
            }
        );
        assert_eq!(
            super::confirmation_outcome(&action, true),
            super::ConfirmationOutcome::PromptRequired {
                reason_code: "confirmation_required"
            }
        );
    }

    #[test]
    fn classifies_route_explain_as_runtime_readonly() {
        let action = CliAction::RouteExplain(crate::cli_commands::route::RouteExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-4o".to_string(),
            client_token_ref: Some("local-client".to_string()),
            output: crate::cli_report::OutputFormat::Table,
        });

        let effect = super::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            super::SideEffectClass::RuntimeReadonly
        );
        assert_eq!(
            effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );
    }

    #[test]
    fn classifies_model_and_client_token_explain_commands_as_runtime_readonly() {
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };
        let actions = [
            CliAction::ModelsList(crate::cli_commands::models::ModelsListOptions {
                connection: connection.clone(),
                client_token_ref: None,
                output: crate::cli_report::OutputFormat::Table,
            }),
            CliAction::ModelsExplain(crate::cli_commands::models::ModelsExplainOptions {
                connection: connection.clone(),
                model: "gpt-4o".to_string(),
                client_token_ref: Some("local-client".to_string()),
                output: crate::cli_report::OutputFormat::Table,
            }),
            CliAction::ClientTokensList(
                crate::cli_commands::client_tokens::ClientTokensListOptions {
                    connection,
                    output: crate::cli_report::OutputFormat::Table,
                },
            ),
        ];

        for action in actions {
            let effect = super::classify_action(&action);

            assert_eq!(
                effect.side_effect_class,
                super::SideEffectClass::RuntimeReadonly
            );
            assert_eq!(
                effect.effect_vector,
                super::EffectVector {
                    reads_local_files: false,
                    reads_management_runtime: true,
                    reads_management_store: false,
                    writes_local_files: false,
                    writes_management_store: false,
                    calls_upstream: false,
                    mutates_runtime: false,
                }
            );
        }
    }

    #[test]
    fn classifies_keys_commands_as_runtime_readonly_store_reads() {
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };
        let actions = [
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::List(
                crate::cli_commands::keys::KeysListOptions {
                    connection: connection.clone(),
                    output: crate::cli_report::OutputFormat::Table,
                },
            )),
            CliAction::Keys(crate::cli_commands::keys::KeysCommand::Stats(
                crate::cli_commands::keys::KeysStatsOptions {
                    connection,
                    credential_set_id: Some("relay-a".to_string()),
                    include_credential_refs: true,
                    credential_ref_limit: 20,
                    output: crate::cli_report::OutputFormat::Table,
                },
            )),
        ];

        for action in actions {
            let effect = super::classify_action(&action);

            assert_eq!(
                effect.side_effect_class,
                super::SideEffectClass::RuntimeReadonly
            );
            assert_eq!(
                effect.effect_vector,
                super::EffectVector {
                    reads_local_files: false,
                    reads_management_runtime: true,
                    reads_management_store: true,
                    writes_local_files: false,
                    writes_management_store: false,
                    calls_upstream: false,
                    mutates_runtime: false,
                }
            );
        }
    }

    #[test]
    fn classifies_keys_import_dry_run_and_confirmed_effects() {
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };

        let dry_run = CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(
            crate::cli_commands::keys::KeysImportOptions {
                connection: connection.clone(),
                credential_set_id: "relay-a".to_string(),
                source: PathBuf::from("data/replacement.keys"),
                mode: crate::cli_commands::keys::KeysImportMode::DryRun,
                output: crate::cli_report::OutputFormat::Table,
            },
        ));
        let confirmed = CliAction::Keys(crate::cli_commands::keys::KeysCommand::Import(
            crate::cli_commands::keys::KeysImportOptions {
                connection,
                credential_set_id: "relay-a".to_string(),
                source: PathBuf::from("data/replacement.keys"),
                mode: crate::cli_commands::keys::KeysImportMode::Apply,
                output: crate::cli_report::OutputFormat::Table,
            },
        ));

        let dry_run_effect = super::classify_action(&dry_run);
        assert_eq!(
            dry_run_effect.side_effect_class,
            super::SideEffectClass::LocalPreview
        );
        assert_eq!(
            dry_run_effect.effect_vector,
            super::EffectVector {
                reads_local_files: true,
                reads_management_runtime: false,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );

        let confirmed_effect = super::classify_action(&confirmed);
        assert_eq!(
            confirmed_effect.side_effect_class,
            super::SideEffectClass::ManagementWrite
        );
        assert_eq!(
            confirmed_effect.effect_vector,
            super::EffectVector {
                reads_local_files: true,
                reads_management_runtime: false,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: true,
                calls_upstream: false,
                mutates_runtime: true,
            }
        );
    }

    #[test]
    fn keys_probe_requires_confirmation_before_upstream_touch() {
        let action = CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(
            crate::cli_commands::keys::KeysProbeOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-a".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                model: "gpt-example".to_string(),
                kind: crate::cli_commands::keys::KeysProbeKind::ModelRetrieve,
                expected_output: None,
                timeout_seconds: None,
                mode: crate::cli_commands::keys::KeysProbeMode::NeedsConfirmation,
                output: crate::cli_report::OutputFormat::Table,
            },
        ));

        assert_eq!(
            super::confirmation_outcome(&action, false),
            super::ConfirmationOutcome::Denied {
                exit_code: 3,
                reason_code: "confirmation_required"
            }
        );
        assert_eq!(
            super::confirmation_outcome(&action, true),
            super::ConfirmationOutcome::PromptRequired {
                reason_code: "confirmation_required"
            }
        );
    }

    #[test]
    fn classifies_keys_probe_dry_run_and_confirmed_effects() {
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };

        let dry_run = CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(
            crate::cli_commands::keys::KeysProbeOptions {
                connection: connection.clone(),
                credential_set_id: "relay-a".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                model: "gpt-example".to_string(),
                kind: crate::cli_commands::keys::KeysProbeKind::ModelRetrieve,
                expected_output: None,
                timeout_seconds: None,
                mode: crate::cli_commands::keys::KeysProbeMode::DryRun,
                output: crate::cli_report::OutputFormat::Table,
            },
        ));
        let confirmed = CliAction::Keys(crate::cli_commands::keys::KeysCommand::Probe(
            crate::cli_commands::keys::KeysProbeOptions {
                connection,
                credential_set_id: "relay-a".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                model: "gpt-example".to_string(),
                kind: crate::cli_commands::keys::KeysProbeKind::ModelRetrieve,
                expected_output: None,
                timeout_seconds: None,
                mode: crate::cli_commands::keys::KeysProbeMode::Apply,
                output: crate::cli_report::OutputFormat::Table,
            },
        ));

        let dry_run_effect = super::classify_action(&dry_run);
        assert_eq!(
            dry_run_effect.side_effect_class,
            super::SideEffectClass::RuntimeReadonly
        );
        assert_eq!(
            dry_run_effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: true,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );

        let confirmed_effect = super::classify_action(&confirmed);
        assert_eq!(
            confirmed_effect.side_effect_class,
            super::SideEffectClass::UpstreamTouching
        );
        assert_eq!(
            confirmed_effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: true,
                calls_upstream: true,
                mutates_runtime: false,
            }
        );
    }

    #[test]
    fn keys_probe_apply_dry_run_is_runtime_readonly_but_confirmed_apply_is_management_write() {
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };

        let plan = CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
            crate::cli_commands::keys::KeysProbeApplyCommand::Plan(
                crate::cli_commands::keys::KeysProbeApplyPlanOptions {
                    connection: connection.clone(),
                    credential_set_id: "relay-a".to_string(),
                    credential_ref: "cr:v1:pos:0".to_string(),
                    output: crate::cli_report::OutputFormat::Table,
                },
            ),
        ));
        let dry_run = CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
            crate::cli_commands::keys::KeysProbeApplyCommand::Apply(
                crate::cli_commands::keys::KeysProbeApplyApplyOptions {
                    connection: connection.clone(),
                    credential_set_id: "relay-a".to_string(),
                    credential_ref: "cr:v1:pos:0".to_string(),
                    probe_result_ref: None,
                    mode: crate::cli_commands::keys::KeysProbeApplyMode::DryRun,
                    output: crate::cli_report::OutputFormat::Table,
                },
            ),
        ));
        let confirmed = CliAction::Keys(crate::cli_commands::keys::KeysCommand::ProbeApply(
            crate::cli_commands::keys::KeysProbeApplyCommand::Apply(
                crate::cli_commands::keys::KeysProbeApplyApplyOptions {
                    connection,
                    credential_set_id: "relay-a".to_string(),
                    credential_ref: "cr:v1:pos:0".to_string(),
                    probe_result_ref: Some("pr:v1:id:1".to_string()),
                    mode: crate::cli_commands::keys::KeysProbeApplyMode::Apply,
                    output: crate::cli_report::OutputFormat::Table,
                },
            ),
        ));

        for action in [plan, dry_run] {
            let effect = super::classify_action(&action);
            assert_eq!(
                effect.side_effect_class,
                super::SideEffectClass::RuntimeReadonly
            );
            assert_eq!(
                effect.effect_vector,
                super::EffectVector {
                    reads_local_files: false,
                    reads_management_runtime: true,
                    reads_management_store: true,
                    writes_local_files: false,
                    writes_management_store: false,
                    calls_upstream: false,
                    mutates_runtime: false,
                }
            );
        }

        let confirmed_effect = super::classify_action(&confirmed);
        assert_eq!(
            confirmed_effect.side_effect_class,
            super::SideEffectClass::ManagementWrite
        );
        assert_eq!(
            confirmed_effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: true,
                writes_local_files: false,
                writes_management_store: true,
                calls_upstream: false,
                mutates_runtime: true,
            }
        );
    }

    #[test]
    fn classifies_failures_commands_as_runtime_readonly_store_reads() {
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };
        let actions = [
            CliAction::FailuresTail(crate::cli_commands::failures::FailureTailOptions {
                connection: connection.clone(),
                last: Some(50),
                filters: crate::cli_commands::failures::FailureFilters::default(),
                output: crate::cli_report::OutputFormat::Table,
            }),
            CliAction::FailuresExplain(crate::cli_commands::failures::FailureExplainOptions {
                connection,
                request_id: "req_123".to_string(),
                last: Some(50),
                filters: crate::cli_commands::failures::FailureFilters::default(),
                output: crate::cli_report::OutputFormat::Table,
            }),
        ];

        for action in actions {
            let effect = super::classify_action(&action);

            assert_eq!(
                effect.side_effect_class,
                super::SideEffectClass::RuntimeReadonly
            );
            assert_eq!(
                effect.effect_vector,
                super::EffectVector {
                    reads_local_files: false,
                    reads_management_runtime: true,
                    reads_management_store: true,
                    writes_local_files: false,
                    writes_management_store: false,
                    calls_upstream: false,
                    mutates_runtime: false,
                }
            );
        }
    }

    #[test]
    fn classifies_doctor_default_as_runtime_readonly_without_store_reads() {
        let action = CliAction::Doctor(crate::cli_commands::doctor::DoctorOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            include_alerts: false,
            include_events: false,
            include_routes: false,
            event_limit: 20,
            output: crate::cli_report::OutputFormat::Table,
        });

        let effect = super::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            super::SideEffectClass::RuntimeReadonly
        );
        assert_eq!(
            effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );
    }

    #[test]
    fn classifies_doctor_alerts_or_events_as_runtime_readonly_store_reads() {
        let base_connection = crate::cli::OperatorConnectionOptions {
            management_url: Some("https://router.example".to_string()),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };
        let actions = [
            CliAction::Doctor(crate::cli_commands::doctor::DoctorOptions {
                connection: base_connection.clone(),
                include_alerts: true,
                include_events: false,
                include_routes: false,
                event_limit: 20,
                output: crate::cli_report::OutputFormat::Table,
            }),
            CliAction::Doctor(crate::cli_commands::doctor::DoctorOptions {
                connection: base_connection,
                include_alerts: false,
                include_events: true,
                include_routes: true,
                event_limit: 20,
                output: crate::cli_report::OutputFormat::Table,
            }),
        ];

        for action in actions {
            let effect = super::classify_action(&action);

            assert_eq!(
                effect.side_effect_class,
                super::SideEffectClass::RuntimeReadonly
            );
            assert!(effect.effect_vector.reads_management_runtime);
            assert!(effect.effect_vector.reads_management_store);
            assert!(!effect.effect_vector.writes_management_store);
            assert!(!effect.effect_vector.calls_upstream);
            assert!(!effect.effect_vector.mutates_runtime);
        }
    }
    #[test]
    fn classifies_models_onboard_plan_dry_run_as_runtime_readonly() {
        let action = CliAction::ModelsOnboardPlan(
            crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                channel_id: "relay-a".to_string(),
                public_model: "coding".to_string(),
                upstream_model: Some("vendor/coding".to_string()),
                mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DryRun,
                output: crate::cli_report::OutputFormat::Json,
            },
        );

        let effect = super::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            super::SideEffectClass::RuntimeReadonly
        );
        assert_eq!(
            effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );
    }

    #[test]
    fn models_onboard_plan_live_variants_are_denied_before_http() {
        let action = CliAction::ModelsOnboardPlan(
            crate::cli_commands::models_onboard::ModelsOnboardPlanOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                channel_id: "relay-a".to_string(),
                public_model: "coding".to_string(),
                upstream_model: None,
                mode: crate::cli_commands::models_onboard::ModelsOnboardPlanMode::DeferredToSeparatePlan,
                output: crate::cli_report::OutputFormat::Json,
            },
        );

        assert_eq!(
            super::confirmation_outcome(&action, false),
            super::ConfirmationOutcome::Denied {
                exit_code: 3,
                reason_code: "deferred_to_separate_plan"
            }
        );
    }

    #[test]
    fn reload_status_cli_is_runtime_readonly() {
        let action = CliAction::ReloadStatus(crate::cli_commands::reload::ReloadStatusOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            output: crate::cli_report::OutputFormat::Json,
        });

        let effect = super::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            super::SideEffectClass::RuntimeReadonly
        );
        assert_eq!(
            effect.effect_vector,
            super::EffectVector {
                reads_local_files: false,
                reads_management_runtime: true,
                reads_management_store: false,
                writes_local_files: false,
                writes_management_store: false,
                calls_upstream: false,
                mutates_runtime: false,
            }
        );
    }
}
