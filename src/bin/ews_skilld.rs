use ews_skill::{ews_client::ntlm_supported, skill::ToolResult, EwsSkill};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Clone)]
enum Transport {
    Unix(PathBuf),
}

#[derive(Debug, Clone)]
struct CliOptions {
    config_path: Option<PathBuf>,
    transport: Transport,
}

fn main() {
    let _log_guard = init_daemon_logging();

    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--check-ntlm") {
        if ntlm_supported() {
            println!("NTLM_SUPPORTED=true");
            std::process::exit(0);
        } else {
            eprintln!("NTLM_SUPPORTED=false");
            std::process::exit(1);
        }
    }

    let options = match parse_cli_options() {
        Ok(opts) => opts,
        Err(e) => {
            error!("invalid cli options: {}", e);
            std::process::exit(2);
        }
    };

    info!("starting ews_skilld");

    let skill = match init_skill(&options) {
        Ok(skill) => skill,
        Err(e) => {
            error!("failed to initialize ews_skilld: {}", e);
            std::process::exit(2);
        }
    };

    let Transport::Unix(socket_path) = options.transport;
    info!(socket = %socket_path.display(), "ews_skilld started (unix socket JSON-RPC)");
    if let Err(e) = run_unix_socket(&skill, &socket_path) {
        error!("unix socket server failed: {}", e);
        std::process::exit(2);
    }

    info!("ews_skilld stopped");
}

fn parse_cli_options() -> Result<CliOptions, String> {
    let mut args = env::args().skip(1);
    let mut config_path: Option<PathBuf> = None;
    let mut socket_path = PathBuf::from("/run/ews-skill/daemon.sock");

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check-ntlm" => {}
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--config requires a path value".to_string())?;
                config_path = Some(PathBuf::from(value));
            }
            "--transport" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--transport requires a value (unix)".to_string())?;
                match value.as_str() {
                    "unix" => {}
                    _ => return Err(format!("unsupported transport: {}", value)),
                }
            }
            "--socket" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--socket requires a path value".to_string())?;
                socket_path = PathBuf::from(value);
            }
            _ => return Err(format!("unknown argument: {}", arg)),
        }
    }

    Ok(CliOptions {
        config_path,
        transport: Transport::Unix(socket_path),
    })
}

fn run_unix_socket(skill: &EwsSkill, socket_path: &Path) -> Result<(), String> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    if socket_path.exists() {
        fs::remove_file(socket_path).map_err(|e| e.to_string())?;
    }

    let listener = UnixListener::bind(socket_path).map_err(|e| e.to_string())?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o660))
        .map_err(|e| e.to_string())?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_unix_client(skill, stream) {
                    warn!("unix client error: {}", e);
                }
            }
            Err(e) => warn!("failed to accept unix socket connection: {}", e),
        }
    }

    Ok(())
}

fn handle_unix_client(skill: &EwsSkill, stream: UnixStream) -> Result<(), String> {
    let reader = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(stream);

    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if bytes == 0 {
            break;
        }

        if line.trim().is_empty() {
            continue;
        }

        let response = parse_and_handle(skill, line.trim());
        write_response(&mut writer, response).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn parse_and_handle(skill: &EwsSkill, raw: &str) -> RpcResponse {
    match serde_json::from_str::<RpcRequest>(raw) {
        Ok(request) => handle_request(skill, request),
        Err(e) => {
            warn!("json-rpc parse error: {}", e);
            rpc_error_response(Value::Null, -32700, format!("parse error: {}", e))
        }
    }
}

fn init_daemon_logging() -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let level = env::var("EWS_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        EnvFilter::new(level)
    });

    if let Ok(file_path) = env::var("EWS_DAEMON_LOG_FILE") {
        let path = PathBuf::from(file_path);
        let parent = path.parent().unwrap_or(Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("ews_skilld.log")
            .to_string();
        let appender = tracing_appender::rolling::never(parent, file_name);
        let (non_blocking, guard) = tracing_appender::non_blocking(appender);
        let _ = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false),
            )
            .with(filter)
            .try_init();
        return Some(guard);
    }

    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .try_init();
    None
}

fn init_skill(options: &CliOptions) -> Result<EwsSkill, String> {
    if let Some(path) = &options.config_path {
        EwsSkill::from_config_file(path)
    } else {
        EwsSkill::from_env()
    }
}

fn handle_request(skill: &EwsSkill, request: RpcRequest) -> RpcResponse {
    if request.jsonrpc.as_deref() != Some("2.0") {
        return rpc_error_response(
            request.id.unwrap_or(Value::Null),
            -32600,
            "invalid request: jsonrpc must be '2.0'".to_string(),
        );
    }

    let id = request.id.unwrap_or(Value::Null);

    info!(method = %request.method, id = %id, "json-rpc request");

    match request.method.as_str() {
        "tools.list" => rpc_result_response(id, json!(EwsSkill::get_tools())),
        "tools.call" => {
            let params = match request.params {
                Some(value) => value,
                None => {
                    return rpc_error_response(
                        id,
                        -32602,
                        "invalid params: expected object with name and args".to_string(),
                    )
                }
            };

            let call = match serde_json::from_value::<ToolCallParams>(params) {
                Ok(value) => value,
                Err(e) => return rpc_error_response(id, -32602, format!("invalid params: {}", e)),
            };

            let args = match call.args {
                Value::Object(_) => call.args,
                Value::Null => json!({}),
                _ => {
                    return rpc_error_response(
                        id,
                        -32602,
                        "invalid params: args must be a JSON object".to_string(),
                    )
                }
            };

            let result = skill.execute_tool(&call.name, args);
            info!(tool = %call.name, success = result.success, "tool call completed");
            rpc_result_response(id, tool_result_to_value(result))
        }
        _ => rpc_error_response(id, -32601, format!("method not found: {}", request.method)),
    }
}

fn tool_result_to_value(result: ToolResult) -> Value {
    let code = if result.success {
        "OK".to_string()
    } else {
        classify_tool_error(result.error.as_deref().unwrap_or(""))
    };

    json!({
        "success": result.success,
        "data": result.data,
        "error": result.error,
        "code": code,
    })
}

fn classify_tool_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("missing required argument") || lower.contains("invalid params") {
        return "E_BAD_ARGS".to_string();
    }
    if lower.contains("unknown tool") {
        return "E_UNKNOWN_TOOL".to_string();
    }
    if lower.contains("auth") || lower.contains("unauthorized") || lower.contains("forbidden") {
        return "E_AUTH".to_string();
    }
    if lower.contains("not found") {
        return "E_NOT_FOUND".to_string();
    }
    if lower.contains("sync") {
        return "E_SYNC".to_string();
    }
    "E_INTERNAL".to_string()
}

fn rpc_result_response(id: Value, result: Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn rpc_error_response(id: Value, code: i32, message: String) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError { code, message }),
    }
}

fn write_response<W: Write>(writer: &mut W, response: RpcResponse) -> io::Result<()> {
    let json = serde_json::to_string(&response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    writer.write_all(json.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}
