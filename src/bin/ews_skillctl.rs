use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use clap::{Args, Parser, Subcommand};
use ews_skill::graph_auth::{login_device_code, logout as graph_logout, GraphAuthConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "ews_skillctl", version, about = "CLI client for ews_skilld")]
struct Cli {
    #[arg(
        long,
        env = "EWS_SOCKET_PATH",
        default_value = "/run/ews-skill/daemon.sock"
    )]
    socket: PathBuf,

    #[arg(long, default_value_t = 10000)]
    timeout_ms: u64,

    #[arg(long)]
    json: bool,

    #[arg(long)]
    human: bool,

    #[arg(long, env = "EWS_CLI_SEARCH_DEFAULT_DAYS", default_value_t = 30)]
    search_default_days: u32,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Login,
    Logout,
    Doctor,
    Tools,
    Health,
    Call(CallArgs),
    List(ListArgs),
    Read(ReadArgs),
    Search(SearchArgs),
    Send(SendArgs),
    Move(MoveArgs),
    Delete(DeleteArgs),
    SyncNow,
    AddFolder(AddFolderArgs),
    Rpc(RpcArgs),
}

#[derive(Args, Debug)]
struct ListArgs {
    #[arg(long, default_value = "inbox")]
    folder: String,
    #[arg(long, default_value_t = 20)]
    limit: i64,
    #[arg(long)]
    unread_only: bool,
}

#[derive(Args, Debug)]
struct ReadArgs {
    #[arg(long)]
    id: String,
}

#[derive(Args, Debug)]
struct SearchArgs {
    #[arg(long)]
    query: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long)]
    sender: Option<String>,
    #[arg(long)]
    date_from: Option<String>,
    #[arg(long)]
    date_to: Option<String>,
    #[arg(long)]
    folder: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: i64,
    #[arg(long)]
    no_date_limit: bool,
    #[arg(long)]
    no_body: bool,
}

#[derive(Args, Debug)]
struct SendArgs {
    #[arg(long)]
    to: String,
    #[arg(long)]
    subject: String,
    #[arg(long)]
    body: String,
}

#[derive(Args, Debug)]
struct MoveArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    folder: String,
}

#[derive(Args, Debug)]
struct DeleteArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    skip_trash: bool,
}

#[derive(Args, Debug)]
struct AddFolderArgs {
    #[arg(long)]
    name: String,
}

#[derive(Args, Debug)]
struct RpcArgs {
    method: String,
    #[arg(long, default_value = "{}")]
    params_json: String,
}

#[derive(Args, Debug)]
struct CallArgs {
    tool: String,
    #[arg(long = "arg")]
    args: Vec<String>,
}

struct Client {
    socket_path: PathBuf,
    timeout: Duration,
}

impl Client {
    fn new(socket_path: PathBuf, timeout_ms: u64) -> Self {
        Self {
            socket_path,
            timeout: Duration::from_millis(timeout_ms.max(1)),
        }
    }

    fn call_method(&self, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        self.send_request(&request)
    }

    fn call_tool(&self, name: &str, args: Value) -> Result<Value, String> {
        let response = self.call_method("tools.call", json!({"name": name, "args": args}))?;
        let result = response
            .get("result")
            .ok_or_else(|| "missing result in response".to_string())?;

        let success = result
            .get("success")
            .and_then(Value::as_bool)
            .ok_or_else(|| "missing success flag in tool response".to_string())?;

        if success {
            Ok(result.get("data").cloned().unwrap_or(Value::Null))
        } else {
            let message = result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("tool call failed");
            Err(message.to_string())
        }
    }

    fn list_tools(&self) -> Result<Value, String> {
        let response = self.call_method("tools.list", json!({}))?;
        response
            .get("result")
            .cloned()
            .ok_or_else(|| "missing tools.list result".to_string())
    }

    fn send_request(&self, request: &Value) -> Result<Value, String> {
        let mut backoff_ms = 50u64;
        let max_retries = 4u8;
        let mut last_err = String::new();

        for attempt in 0..=max_retries {
            match self.send_request_once(request) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = e;
                    if attempt < max_retries && is_retryable_socket_error(&last_err) {
                        sleep(Duration::from_millis(backoff_ms));
                        backoff_ms = (backoff_ms * 2).min(600);
                        continue;
                    }
                    break;
                }
            }
        }

        Err(last_err)
    }

    fn send_request_once(&self, request: &Value) -> Result<Value, String> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| e.to_string())?;

        let body = serde_json::to_string(request).map_err(|e| e.to_string())?;
        stream
            .write_all(body.as_bytes())
            .map_err(|e| e.to_string())?;
        stream.write_all(b"\n").map_err(|e| e.to_string())?;
        stream.flush().map_err(|e| e.to_string())?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response).map_err(|e| e.to_string())?;
        if response.trim().is_empty() {
            return Err("empty response from daemon".to_string());
        }

        serde_json::from_str(response.trim()).map_err(|e| e.to_string())
    }
}

fn is_retryable_socket_error(message: &str) -> bool {
    message.contains("Resource temporarily unavailable")
        || message.contains("temporarily unavailable")
        || message.contains("WouldBlock")
}

fn output_json(cli: &Cli) -> bool {
    cli.json || !cli.human
}

fn print_output(as_json: bool, text: &str, payload: Value) {
    if as_json {
        println!("{}", payload);
    } else {
        println!("{}", text);
    }
}

fn parse_cli_args(input: &[String]) -> Result<Value, String> {
    let mut out = serde_json::Map::new();
    for raw in input {
        let (key, value) = raw
            .split_once('=')
            .ok_or_else(|| format!("invalid --arg '{}', expected key=value", raw))?;
        if key.trim().is_empty() {
            return Err(format!("invalid --arg '{}', key is empty", raw));
        }
        out.insert(key.to_string(), parse_scalar_json(value));
    }
    Ok(Value::Object(out))
}

fn parse_scalar_json(value: &str) -> Value {
    if value.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if value.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    if let Ok(n) = value.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = value.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    if (value.starts_with('{') && value.ends_with('}'))
        || (value.starts_with('[') && value.ends_with(']'))
    {
        if let Ok(parsed) = serde_json::from_str::<Value>(value) {
            return parsed;
        }
    }
    Value::String(value.to_string())
}

fn main() {
    let cli = Cli::parse();
    let as_json = output_json(&cli);
    let Some(command) = cli.command else {
        eprintln!("No command provided. Run `ews_skillctl --help`.");
        std::process::exit(2);
    };

    match command {
        Command::Login => {
            let auth = graph_auth_from_env().unwrap_or_else(|e| {
                eprintln!("{}", e);
                std::process::exit(2);
            });
            if let Err(e) = login_device_code(&auth) {
                if as_json {
                    println!("{}", json!({"ok": false, "error": e}));
                } else {
                    eprintln!("{}", e);
                }
                std::process::exit(1);
            }
            if as_json {
                println!(
                    "{}",
                    json!({"ok": true, "message": "graph delegated login successful"})
                );
            } else {
                println!("graph delegated login successful");
            }
            return;
        }
        Command::Logout => {
            if let Err(e) = graph_logout() {
                if as_json {
                    println!("{}", json!({"ok": false, "error": e}));
                } else {
                    eprintln!("{}", e);
                }
                std::process::exit(1);
            }
            if as_json {
                println!(
                    "{}",
                    json!({"ok": true, "message": "graph delegated token cache cleared"})
                );
            } else {
                println!("graph delegated token cache cleared");
            }
            return;
        }
        _ => {}
    }

    let client = Client::new(cli.socket.clone(), cli.timeout_ms);

    let result = match command {
        Command::Login | Command::Logout => unreachable!(),
        Command::Doctor => {
            let tools = client.list_tools();
            let health = client.call_tool("email_health", json!({}));
            match (tools, health) {
                (Ok(t), Ok(h)) => {
                    let payload = json!({"ok": true, "socket": cli.socket, "tools": t, "health": h});
                    print_output(as_json, "doctor: ok", payload);
                    Ok(())
                }
                (t, h) => {
                    Err(format!(
                        "doctor failed: tools_error={:?}, health_error={:?}",
                        t.err(),
                        h.err()
                    ))
                }
            }
        }
        Command::Tools => match client.list_tools() {
            Ok(data) => {
                if as_json {
                    println!("{}", data);
                } else {
                    println!("tools listed");
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::Health => match client.call_tool("email_health", json!({})) {
            Ok(data) => {
                print_output(as_json, "health: ok", data);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::Call(args) => {
            match parse_cli_args(&args.args) {
                Ok(tool_args) => match client.call_tool(&args.tool, tool_args) {
                    Ok(data) => {
                        if as_json {
                            println!("{}", data);
                        } else {
                            println!("call: {} ok", args.tool);
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        Command::List(args) => match client.call_tool(
            "email_list",
            json!({"folder_name": args.folder, "limit": args.limit, "unread_only": args.unread_only}),
        ) {
            Ok(data) => {
                if as_json {
                    println!("{}", data);
                } else {
                    let count = data
                        .get("emails")
                        .and_then(Value::as_array)
                        .map(|v| v.len())
                        .unwrap_or(0);
                    println!("emails: {}", count);
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::Read(args) => match client.call_tool("email_read", json!({"email_id": args.id})) {
            Ok(data) => {
                if as_json {
                    println!("{}", data);
                } else {
                    let subject = data
                        .get("subject")
                        .and_then(Value::as_str)
                        .unwrap_or("<no subject>");
                    println!("read: {}", subject);
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::Search(args) => {
            let mut date_from = args.date_from;
            let date_to = args.date_to;
            if !args.no_date_limit
                && date_from.is_none()
                && date_to.is_none()
                && cli.search_default_days > 0
            {
                let cutoff = Utc::now() - ChronoDuration::days(cli.search_default_days as i64);
                date_from = Some(cutoff.to_rfc3339_opts(SecondsFormat::Secs, true));
            }

            match client.call_tool(
                "email_search",
                json!({
                    "query": args.query,
                    "subject": args.subject,
                    "sender": args.sender,
                    "date_from": date_from,
                    "date_to": date_to,
                    "folder_name": args.folder,
                    "limit": args.limit,
                    "include_body": !args.no_body,
                }),
            ) {
                Ok(data) => {
                    if as_json {
                        println!("{}", data);
                    } else {
                    let count = data
                        .get("results")
                        .and_then(Value::as_array)
                        .map(|v| v.len())
                        .unwrap_or(0);
                    println!("results: {}", count);
                }
                Ok(())
            }
            Err(e) => Err(e),
            }
        }
        Command::Send(args) => match client.call_tool(
            "email_send",
            json!({"to": args.to, "subject": args.subject, "body": args.body}),
        ) {
            Ok(data) => {
                print_output(as_json, "send: ok", data);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::Move(args) => match client.call_tool(
            "email_move",
            json!({"email_id": args.id, "destination_folder": args.folder}),
        ) {
            Ok(data) => {
                print_output(as_json, "move: ok", data);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::Delete(args) => match client.call_tool(
            "email_delete",
            json!({"email_id": args.id, "skip_trash": args.skip_trash}),
        ) {
            Ok(data) => {
                print_output(as_json, "delete: ok", data);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::SyncNow => match client.call_tool("email_sync_now", json!({})) {
            Ok(data) => {
                print_output(as_json, "sync-now: ok", data);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Command::AddFolder(args) => {
            match client.call_tool("email_add_folder", json!({"folder_name": args.name})) {
                Ok(data) => {
                    print_output(as_json, "add-folder: ok", data);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Command::Rpc(args) => {
            let params: Result<Value, String> =
                serde_json::from_str(&args.params_json).map_err(|e| format!("invalid params-json: {}", e));
            match params {
                Ok(p) => match client.call_method(&args.method, p) {
                    Ok(data) => {
                        println!("{}", data);
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
    };

    if let Err(e) = result {
        if as_json {
            println!("{}", json!({"ok": false, "error": e}));
        } else {
            eprintln!("{}", e);
        }
        std::process::exit(1);
    }
}

fn graph_auth_from_env() -> Result<GraphAuthConfig, String> {
    let tenant_id = std::env::var("GRAPH_TENANT_ID")
        .map_err(|_| "missing GRAPH_TENANT_ID in environment".to_string())?;
    let client_id = std::env::var("GRAPH_CLIENT_ID")
        .map_err(|_| "missing GRAPH_CLIENT_ID in environment".to_string())?;
    Ok(GraphAuthConfig {
        tenant_id,
        client_id,
    })
}
