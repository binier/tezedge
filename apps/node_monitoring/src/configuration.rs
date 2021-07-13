// Copyright (c) SimpleStaking, Viable Systems and Tezedge Contributors
// SPDX-License-Identifier: MIT

use clap::{App, Arg};
use std::fs;
use std::path::Path;
use std::{env, fmt};

use crate::node::{Node, NodeType};

#[derive(Clone, Debug)]
pub struct DeployMonitoringEnvironment {
    // logging level
    pub log_level: slog::Level,

    // interval in seconds to check for new remote image
    // pub image_monitor_interval: Option<u64>,

    // interval in seconds to check for new remote image
    pub resource_monitor_interval: u64,

    // rpc server port
    pub rpc_port: u16,

    // flag for sandbox mode
    // pub is_sandbox: bool,

    // Path for the compose file needed to manage the deployed containers
    // pub compose_file_path: PathBuf,

    // Thresholds to alerts
    pub tezedge_alert_thresholds: AlertThresholds,

    // Thresholds to alerts
    pub ocaml_alert_thresholds: AlertThresholds,

    // flag for volume cleanup mode
    // pub cleanup_volumes: bool,
    pub slack_configuration: Option<SlackConfiguration>,

    // pub tezedge_only: bool,

    // pub disable_debugger: bool,
    pub tezedge_volume_path: String,

    pub nodes: Vec<Node>,
}

#[derive(Clone, Copy, Debug)]
pub struct AlertThresholds {
    pub memory: u64,
    pub disk: u64,
    pub synchronization: i64,
    pub cpu: Option<u64>,
}

impl fmt::Display for AlertThresholds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\n\tMemory: {}MB\n\tTotal disk space: {}%\n\tCpu: {:?}%\n\tSynchronization: {}s\n",
            self.memory / 1024 / 1024,
            self.disk,
            self.cpu,
            self.synchronization,
        )
    }
}

#[derive(Clone, Debug)]
pub struct SlackConfiguration {
    // slack bot token
    pub slack_token: String,

    // slack channel url
    pub slack_url: String,

    // slack channel name
    pub slack_channel_name: String,
}

fn deploy_monitoring_app() -> App<'static, 'static> {
    let app = App::new("Tezedge node monitoring app")
        .version("1.7.0")
        .author("SimpleStaking and the project contributors")
        .setting(clap::AppSettings::AllArgsOverrideSelf)
        .arg(
            Arg::with_name("config-file")
                .long("config-file")
                .takes_value(true)
                .value_name("PATH")
                .help("Configuration file with start-up arguments (same format as cli arguments)")
                .validator(|v| {
                    if Path::new(&v).exists() {
                        Ok(())
                    } else {
                        Err(format!("Configuration file not found at '{}'", v))
                    }
                }),
        )
        .arg(Arg::with_name("tezedge-nodes")
            .long("tezedge-nodes")
            .takes_value(true)
            .multiple(true)
            .value_name("TAG:PORT")
            .help("The tagged tezedge nodes to moitor running on this system. Format: TAG1:PORT1,TAG2:PORT2,TAG3:PORT3")
        )
        .arg(Arg::with_name("ocaml-nodes")
            .long("ocaml-nodes")
            .takes_value(true)
            .multiple(true)
            .value_name("TAG:PORT")
            .help("The tagged ocaml nodes to moitor running on this system. Format: TAG1:PORT1,TAG2:PORT2,TAG3:PORT3")
        )
        .arg(
            Arg::with_name("log-level")
                .long("log-level")
                .takes_value(true)
                .value_name("LEVEL")
                .possible_values(&["critical", "error", "warn", "info", "debug", "trace"])
                .help("Set log level"),
        )
        .arg(
            Arg::with_name("slack-token")
                .long("slack-token")
                .takes_value(true)
                .value_name("SLACK-TOKEN")
                .help("The slack token of the bot sending messages"),
        )
        .arg(
            Arg::with_name("slack-url")
                .long("slack-url")
                .takes_value(true)
                .value_name("SLACK-URL")
                .help("The slack url of the channel to send the messages to"),
        )
        .arg(
            Arg::with_name("slack-channel-name")
                .long("slack-channel-name")
                .takes_value(true)
                .value_name("SLACK-CHANNEL-NAME")
                .help("The slack url of the channel to send the messages to"),
        )
        .arg(
            Arg::with_name("resource-monitor-interval")
                .long("resource-monitor-interval")
                .takes_value(true)
                .value_name("RESOURCE-MONITOR-INTERVAL")
                .help("Interval in seconds to take resource utilization measurements"),
        )
        .arg(
            Arg::with_name("rpc-port")
                .long("rpc-port")
                .takes_value(true)
                .value_name("RPC-PORT")
                .help("Port number to open the monitoring rpc server on"),
        )
        .arg(
            Arg::with_name("tezedge-alert-threshold-disk")
                .long("tezedge-alert-threshold-disk")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-DISK")
                .help("Thershold in bytes for critical alerts - disk"),
        )
        .arg(
            Arg::with_name("tezedge-alert-threshold-memory")
                .long("tezedge-alert-threshold-memory")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-MEMORY")
                .help("Thershold in bytes for critical alerts - memory"),
        )
        .arg(
            Arg::with_name("tezedge-alert-threshold-cpu")
                .long("tezedge-alert-threshold-cpu")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-CPU")
                .help("Thershold in % for critical alerts - cpu"),
        )
        .arg(
            Arg::with_name("tezedge-alert-threshold-synchronization")
                .long("tezedge-alert-threshold-synchronization")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-SYNCHRONIZATION")
                .help("Thershold in seconds for critical alerts - synchronization"),
        )
        .arg(
            Arg::with_name("ocaml-alert-threshold-disk")
                .long("ocaml-alert-threshold-disk")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-DISK")
                .help("Thershold in bytes for critical alerts - disk"),
        )
        .arg(
            Arg::with_name("ocaml-alert-threshold-memory")
                .long("ocaml-alert-threshold-memory")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-MEMORY")
                .help("Thershold in bytes for critical alerts - memory"),
        )
        .arg(
            Arg::with_name("ocaml-alert-threshold-cpu")
                .long("ocaml-alert-threshold-cpu")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-CPU")
                .help("Thershold in % for critical alerts - cpu"),
        )
        .arg(
            Arg::with_name("ocaml-alert-threshold-synchronization")
                .long("ocaml-alert-threshold-synchronization")
                .takes_value(true)
                .value_name("ALERT-THRESHOLD-SYNCHRONIZATION")
                .help("Thershold in seconds for critical alerts - synchronization"),
        );
    app
}

// Validates single required arg. If missing, exit whole process
pub fn validate_required_arg(args: &clap::ArgMatches, arg_name: &str) {
    if !args.is_present(arg_name) {
        panic!("required \"{}\" arg is missing !!!", arg_name);
    }
}

fn validate_required_args(args: &clap::ArgMatches) {
    if !args.is_present("tezedge-nodes") {
        validate_required_arg(args, "ocaml-nodes");
    }

    if !args.is_present("ocaml-nodes") {
        validate_required_arg(args, "tezedge-nodes");
    }
}

fn check_slack_args(args: &clap::ArgMatches) -> Option<SlackConfiguration> {
    // if any of the slack args are present, all 3 of them must be present
    if args.is_present("slack-token")
        || args.is_present("slack-channel-name")
        || args.is_present("slack-url")
    {
        validate_required_arg(args, "slack-token");
        validate_required_arg(args, "slack-channel-name");
        validate_required_arg(args, "slack-url");

        Some(SlackConfiguration {
            slack_token: args.value_of("slack-token").unwrap_or("").to_string(),
            slack_url: args.value_of("slack-url").unwrap_or("").to_string(),
            slack_channel_name: args
                .value_of("slack-channel-name")
                .unwrap_or("")
                .to_string(),
        })
    } else {
        None
    }
}

impl DeployMonitoringEnvironment {
    pub fn from_args() -> Self {
        let app = deploy_monitoring_app();
        let args = app.clone().get_matches();

        validate_required_args(&args);
        let slack_configuration = check_slack_args(&args);

        let tezedge_alert_thresholds = AlertThresholds {
            memory: args
                .value_of("tezedge-alert-threshold-memory")
                .unwrap_or("4096")
                .parse::<u64>()
                .expect("Was expecting number of megabytes [u64]")
                * 1024
                * 1024,
            disk: args
                .value_of("tezedge-alert-threshold-disk")
                .unwrap_or("95")
                .parse::<u64>()
                .expect("Was expecting percentage [u64]"),
            synchronization: args
                .value_of("tezedge-alert-threshold-synchronization")
                .unwrap_or("300")
                .parse::<i64>()
                .expect("Was seconds [i64]"),
            cpu: args
                .value_of("tezedge-alert-threshold-cpu")
                .map(|cpu_thresh| {
                    cpu_thresh
                        .parse::<u64>()
                        .expect("Was expecting percentage [u64]")
                }),
        };

        let tezedge_volume_path =
            env::var("TEZEDGE_VOLUME_PATH").unwrap_or("/tmp/deploy_monitoring/tezedge".to_string());

        if !Path::new(&tezedge_volume_path).exists() {
            fs::create_dir_all(&tezedge_volume_path).unwrap_or_else(|_| {
                panic!("Failed to create directory: {:?}", &tezedge_volume_path)
            });
        }

        let ocaml_alert_thresholds = AlertThresholds {
            memory: args
                .value_of("ocaml-alert-threshold-memory")
                .unwrap_or("6144")
                .parse::<u64>()
                .expect("Was expecting number of megabytes [u64]")
                * 1024
                * 1024,
            disk: args
                .value_of("ocaml-alert-threshold-disk")
                .unwrap_or("95")
                .parse::<u64>()
                .expect("Was expecting percentage [u64]"),
            synchronization: args
                .value_of("ocaml-alert-threshold-synchronization")
                .unwrap_or("300")
                .parse::<i64>()
                .expect("Was seconds [i64]"),
            cpu: args
                .value_of("ocaml-alert-threshold-cpu")
                .map(|cpu_thresh| {
                    cpu_thresh
                        .parse::<u64>()
                        .expect("Was expecting percentage [u64]")
                }),
        };

        let tezedge_volume_path = env::var("TEZEDGE_VOLUME_PATH").unwrap_or(
            "/var/lib/docker/volumes/deploy_monitoring_tezedge-shared-data/_data".to_string(),
        );

        if !Path::new(&tezedge_volume_path).exists() {
            fs::create_dir_all(&tezedge_volume_path).unwrap_or_else(|_| {
                panic!("Failed to create directory: {:?}", &tezedge_volume_path)
            });
        }

        let mut tezedge_nodes: Vec<Node> = if let Some(vals) = args.values_of("tezedge-nodes") {
            vals.into_iter()
                .map(|val| {
                    let components: Vec<&str> = val.split(":").collect();
                    if components.len() == 2 {
                        let port = components[1]
                            .parse::<u16>()
                            .expect("Expected valid port number");
                        let tag = components[0].to_string();
                        Node::new(port, tag, None, "DUMMy".to_string(), NodeType::Tezedge)
                    } else {
                        // TODO: rework probably
                        panic!("Wrong format!!!")
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let ocaml_nodes: Vec<Node> = if let Some(vals) = args.values_of("ocaml-nodes") {
            vals.into_iter()
                .map(|val| {
                    let components: Vec<&str> = val.split(":").collect();
                    if components.len() == 2 {
                        let port = components[1]
                            .parse::<u16>()
                            .expect("Expected valid port number");
                        let tag = components[0].to_string();
                        Node::new(port, tag, None, "DUMMy".to_string(), NodeType::Ocaml)
                    } else {
                        // TODO: rework probably
                        panic!("Wrong format!!!")
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        tezedge_nodes.extend(ocaml_nodes);

        DeployMonitoringEnvironment {
            log_level: args
                .value_of("log-level")
                .unwrap_or("info")
                .parse::<slog::Level>()
                .expect("Was expecting one value from slog::Level"),
            resource_monitor_interval: args
                .value_of("resource-monitor-interval")
                .unwrap_or("5")
                .parse::<u64>()
                .expect("Expected u64 value of seconds"),
            rpc_port: args
                .value_of("rpc-port")
                .unwrap_or("38732")
                .parse::<u16>()
                .expect("Expected u16 value of valid port number"),
            tezedge_alert_thresholds,
            ocaml_alert_thresholds,
            slack_configuration,
            tezedge_volume_path,
            nodes: tezedge_nodes,
        }
    }
}
