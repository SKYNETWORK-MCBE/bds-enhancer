pub mod action;
pub mod color;
pub mod consts;
pub mod log_level;
pub mod sourcemap_resolver;
pub mod stream;

use json::{self, object};
use regex::Regex;
use std::env;
use std::io::Write;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use action::Action;
use color::{ANSI_DIM, ANSI_NORMAL_INTENSITY, Color};
use consts::LOG_PREFIX;
use log_level::LogLevel;
use sourcemap_resolver::SourcemapResolver;
use stream::LogDelimiterStream;

lazy_static::lazy_static! {
    static ref ACTION_MESSAGE_REGEX: Regex = Regex::new(r".*\[Scripting\] bds_enhancer:(?P<json>\{.*\})").unwrap();
    static ref LOG_REGEX: Regex = Regex::new(&format!(r"{} (?P<level>(INFO|WARN|ERROR))\] ", LOG_PREFIX)).unwrap();
    static ref ON_JOIN_REGEX: Regex = Regex::new(r"Player connected: (?P<player>.+), xuid: (?P<xuid>\d+)").unwrap();
    static ref ON_SPAWN_REGEX: Regex = Regex::new(r"Player Spawned: (?P<player>.+) xuid: (?P<xuid>\d+), pfid: (?P<pfid>.+)").unwrap();
}

fn handle_child_stdin(rx: Receiver<String>, mut child_stdin: ChildStdin) {
    loop {
        let input = rx.recv().unwrap();
        child_stdin
            .write_all(input.as_bytes())
            .expect("Failed to write to stdin");
    }
}

fn handle_stdin(child_stdin: Sender<String>) {
    let stdin = std::io::stdin();

    loop {
        let mut line = String::new();
        stdin.read_line(&mut line).unwrap();

        child_stdin.send(line).unwrap();
    }
}

fn get_log_level(log: &str) -> LogLevel {
    let level = LOG_REGEX
        .captures(log)
        .map(|caps| caps["level"].to_string())
        .unwrap_or("INFO".to_string());

    level.parse().unwrap()
}

fn parse_action(log: &str) -> Option<Action> {
    let caps = ACTION_MESSAGE_REGEX.captures(log)?;

    let json = caps.name("json").unwrap().as_str();
    serde_json::from_str(json).ok()?
}

enum IncomingLog<'a> {
    Action(Action),
    Output { level: LogLevel, original: &'a str },
}

fn classify_log(log: &str) -> IncomingLog<'_> {
    if let Some(action) = parse_action(log) {
        return IncomingLog::Action(action);
    }

    IncomingLog::Output {
        level: get_log_level(log),
        original: log.strip_prefix("NO LOG FILE! - ").unwrap_or(log),
    }
}

fn handle_action(child_stdin: &Sender<String>, action: Action, command_status: &mut CommandStatus) {
    match action {
        Action::Transfer(arg) => execute_command(
            child_stdin,
            format!("transfer {} {} {}", arg.player, arg.host, arg.port),
        ),
        Action::Kick(arg) => {
            execute_command(child_stdin, format!("kick {} {}", arg.player, arg.reason))
        }
        Action::Reload => execute_command(child_stdin, "reload".to_string()),
        Action::Stop => execute_command(child_stdin, "stop".to_string()),
        Action::Execute(arg) => {
            if arg.result {
                command_status.waiting = true;
                command_status.command = arg.command.clone();
                command_status.scriptevent = "bds_enhancer:result".to_string();
            }
            execute_command(child_stdin, arg.command.to_string());
        }
        Action::ExecuteShell(arg) => {
            let result = execute_shell_command(&arg.main_command.clone(), arg.args.clone());
            match result {
                Ok(result) => {
                    for i in 0..result.trim().chars().count() / 1500 + 1 {
                        let result_tmp = result
                            .trim()
                            .chars()
                            .skip(i * 1500)
                            .take(1500)
                            .collect::<String>();
                        let result: json::JsonValue = object! {
                            "command" => arg.main_command.clone() + " " + &arg.args.clone().join(" "),
                            "result_message" => result_tmp.clone(),
                            "count" => i,
                            "end" => i == result.trim().chars().count() / 1500,
                            "err" => false,
                        };
                        execute_command(
                            child_stdin,
                            format!(
                                "scriptevent {} {}",
                                "bds_enhancer:shell_result",
                                result.dump()
                            ),
                        );
                    }
                }
                Err(e) => {
                    if arg.result {
                        let return_value = object! {
                            "command" => arg.main_command + " " + &arg.args.clone().join(" "),
                            "result_message" => format!("Error: {}", e),
                            "err" => true,
                        };
                        execute_command(
                            child_stdin,
                            format!(
                                "scriptevent {} {}",
                                "bds_enhancer:shell_result",
                                return_value.dump()
                            ),
                        );
                    }
                }
            }
        }
    }
}

fn custom_handler(log: &str, child_stdin: &Sender<String>) {
    if let Some(caps) = ON_JOIN_REGEX.captures(log) {
        let player = caps.name("player").unwrap().as_str();
        let xuid = caps.name("xuid").unwrap().as_str();
        execute_command(
            child_stdin,
            format!("scriptevent system:on_join {}|{}", player, xuid),
        );
    } else if let Some(caps) = ON_SPAWN_REGEX.captures(log) {
        let player = caps.name("player").unwrap().as_str();
        let xuid = caps.name("xuid").unwrap().as_str();
        let pfid = caps.name("pfid").unwrap().as_str();
        execute_command(
            child_stdin,
            format!("scriptevent system:on_spawn {}|{}|{}", player, xuid, pfid),
        );
    }
}

fn forward_command_result(
    log: &str,
    child_stdin: &Sender<String>,
    command_status: &mut CommandStatus,
) {
    if !command_status.waiting {
        return;
    }

    for i in 0..(log.chars().count() / 1500 + 1) {
        let result_tmp = log.chars().skip(i * 1500).take(1500).collect::<String>();
        let result: json::JsonValue = object! {
            "command" => command_status.command.clone(),
            "result_message" => result_tmp,
            "count" => i,
            "end" => i == log.chars().count() / 1500,
        };
        execute_command(
            child_stdin,
            format!(
                "scriptevent {} {} ",
                command_status.scriptevent,
                result.dump()
            ),
        );
    }
    command_status.waiting = false;
}

fn handle_child_stdout(
    child_stdin: Sender<String>,
    child_stdout: ChildStdout,
    command_status: &mut CommandStatus,
    sourcemap_resolver: &SourcemapResolver,
) {
    let logs = LogDelimiterStream::new(child_stdout);
    let mut stdout = std::io::stdout();

    for log in logs {
        let (level, log) = match classify_log(&log) {
            IncomingLog::Action(action) => {
                handle_action(&child_stdin, action, command_status);
                continue;
            }
            IncomingLog::Output { level, original } => (level, original),
        };
        forward_command_result(log, &child_stdin, command_status);
        let display_log = sourcemap_resolver.resolve_log_with_generated_style(
            log,
            ANSI_DIM,
            ANSI_NORMAL_INTENSITY,
        );
        let _ = stdout
            .write(format!("{}{}{}\n", level.to_color(), display_log, Color::Reset).as_bytes());

        // Event detection must continue to use the untouched BDS log.
        custom_handler(log, &child_stdin);
    }
}

fn execute_command(child_stdin: &Sender<String>, command: String) {
    child_stdin.send(format!("{}\n", command)).unwrap();
}

fn execute_shell_command(command: &str, args: Vec<String>) -> Result<String, std::io::Error> {
    let output = Command::new(command).args(args).output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(stdout.to_string())
        }
        Err(e) => Err(e),
    }
}

fn build_command(os: &str, cwd: &str, executable_name: &str) -> Command {
    if os != "linux" && os != "windows" {
        panic!("Unsupported platform: {}", os);
    }

    let mut command = Command::new(Path::new(cwd).join(executable_name));

    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());

    if os == "linux" {
        command.env("LD_LIBRARY_PATH", ".");
    }

    command
}

fn main() {
    println!(
        "{}[bds-enhancer]{} bds-enhancer v{}",
        Color::Green,
        Color::Reset,
        env!("CARGO_PKG_VERSION")
    );
    let os = env::consts::OS;
    let cwd = env::args().nth(1).unwrap_or(".".to_string());
    let executable_name = env::args().nth(2).unwrap_or("bedrock_server".to_string());
    let sourcemap_resolver = SourcemapResolver::discover(Path::new(&cwd));

    println!(
        "{}[bds-enhancer]{} Starting {}...",
        Color::Green,
        Color::Reset,
        executable_name
    );
    let mut child = build_command(os, &cwd, &executable_name)
        .spawn()
        .expect("Failed to spawn process");

    let child_stdin = child.stdin.take().expect("Failed to get stdin");
    let stdout = child.stdout.take().expect("Failed to get stdout");

    let (tx, rx) = channel::<String>();
    let tx2 = tx.clone();

    thread::spawn(move || handle_child_stdin(rx, child_stdin));
    thread::spawn(move || handle_stdin(tx));

    let mut command_status = CommandStatus {
        waiting: false,
        command: "".to_string(),
        scriptevent: "".to_string(),
    };
    handle_child_stdout(tx2, stdout, &mut command_status, &sourcemap_resolver);
    let _ = child.wait();
}

struct CommandStatus {
    waiting: bool,
    command: String,
    scriptevent: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_logs_are_classified_before_console_rendering() {
        let log = r#"[2026 INFO] [Scripting] bds_enhancer:{"action":"reload"}"#;

        assert!(matches!(
            classify_log(log),
            IncomingLog::Action(Action::Reload)
        ));
    }

    #[test]
    fn output_logs_keep_the_original_text_and_level() {
        let log = "NO LOG FILE! - [2026-08-19 16:00:00:000 ERROR] [Scripting] probe";

        let (level, original) = match classify_log(log) {
            IncomingLog::Output { level, original } => (level, original),
            IncomingLog::Action(_) => panic!("ordinary logs must be rendered"),
        };
        assert_eq!(level.as_str(), "ERROR");
        assert_eq!(
            original,
            "[2026-08-19 16:00:00:000 ERROR] [Scripting] probe"
        );
    }

    #[test]
    fn custom_handlers_still_receive_original_join_and_spawn_logs() {
        let (sender, receiver) = channel();
        custom_handler("Player connected: Steve, xuid: 123", &sender);
        custom_handler(
            "Player Spawned: Alex xuid: 456, pfid: test-platform",
            &sender,
        );

        assert_eq!(
            receiver.recv().expect("join event command must be emitted"),
            "scriptevent system:on_join Steve|123\n"
        );
        assert_eq!(
            receiver
                .recv()
                .expect("spawn event command must be emitted"),
            "scriptevent system:on_spawn Alex|456|test-platform\n"
        );
    }

    #[test]
    fn command_results_forward_the_original_log() {
        let (sender, receiver) = channel();
        let mut status = CommandStatus {
            waiting: true,
            command: "list".to_owned(),
            scriptevent: "bds_enhancer:result".to_owned(),
        };
        let original = "[2026-08-19 16:00:00:000 INFO] There are 0/10 players online";

        forward_command_result(original, &sender, &mut status);

        let command = receiver
            .recv()
            .expect("command result event must be emitted");
        let payload = command
            .strip_prefix("scriptevent bds_enhancer:result ")
            .expect("result event prefix must be preserved")
            .trim();
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("result payload must remain valid JSON");
        assert_eq!(payload["command"], "list");
        assert_eq!(payload["result_message"], original);
        assert_eq!(payload["count"], 0);
        assert_eq!(payload["end"], true);
        assert!(!status.waiting);
    }

    #[test]
    fn unrelated_logs_are_unchanged_by_an_empty_resolver() {
        let resolver = SourcemapResolver::default();
        let log = "[2026 INFO] Server started.";

        assert_eq!(resolver.resolve_log(log), log);
    }
}
