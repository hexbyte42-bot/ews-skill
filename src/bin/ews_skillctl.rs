use std::env;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn main() {
    let socket_path = match parse_socket_path() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("invalid arguments: {}", e);
            std::process::exit(2);
        }
    };

    let stdin = io::stdin();
    let mut stdout = BufWriter::new(io::stdout());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(value) => value,
            Err(e) => {
                let _ = writeln!(stdout, "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":-32000,\"message\":\"stdin read failed: {}\"}}}}", e);
                let _ = stdout.flush();
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        match forward_request(&socket_path, &line) {
            Ok(response) => {
                let _ = writeln!(stdout, "{}", response);
                let _ = stdout.flush();
            }
            Err(e) => {
                let escaped = e.replace('"', "'");
                let _ = writeln!(
                    stdout,
                    "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":-32001,\"message\":\"daemon communication failed: {}\"}}}}",
                    escaped
                );
                let _ = stdout.flush();
            }
        }
    }
}

fn parse_socket_path() -> Result<PathBuf, String> {
    let mut args = env::args().skip(1);
    let mut socket = env::var("EWS_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/run/ews-skill/daemon.sock"));

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--socket requires a path value".to_string())?;
                socket = PathBuf::from(value);
            }
            _ => return Err(format!("unknown argument: {}", arg)),
        }
    }

    Ok(socket)
}

fn forward_request(socket_path: &PathBuf, request: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket_path).map_err(|e| e.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(|e| e.to_string())?;
    if response.trim().is_empty() {
        return Err("empty response from daemon".to_string());
    }
    Ok(response.trim().to_string())
}
