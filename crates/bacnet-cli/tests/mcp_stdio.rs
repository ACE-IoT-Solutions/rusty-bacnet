//! `bacnet mcp serve --stdio` as an agent host launches it: JSON-RPC on
//! stdin and stdout, logs on stderr only (#2066).
#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Run the server with `args`, send `requests` (one JSON-RPC message a
/// line), close stdin, and return its stdout lines and stderr.
fn session(args: &[&str], requests: &[Value]) -> (Vec<String>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bacnet"))
        .args(args)
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut lines = stdout.lines();
    let mut out = Vec::new();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        // A request with an id gets its answer before the next goes.
        if request.get("id").is_some() {
            loop {
                let line = lines.next().expect("an answer").unwrap();
                let done = serde_json::from_str::<Value>(line.trim_end_matches('\r'))
                    .is_ok_and(|v| v["id"] == request["id"]);
                out.push(line);
                if done {
                    break;
                }
            }
        }
    }
    drop(stdin);
    out.extend(lines.map(Result::unwrap));
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "exit {:?}", output.status);
    (out, String::from_utf8_lossy(&output.stderr).into_owned())
}

fn requests() -> Vec<Value> {
    vec![
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": { "name": "test-host", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "bacnet_discover", "arguments": { "send": false } } }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "prompts/list" }),
    ]
}

/// Every stdout line is a JSON-RPC message, even with debug logging on.
fn assert_json_rpc_only(lines: &[String]) {
    assert!(lines.len() >= 4, "{lines:?}");
    for line in lines {
        let value: Value = serde_json::from_str(line.trim_end_matches('\r'))
            .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
        assert_eq!(value["jsonrpc"], json!("2.0"), "{line}");
    }
}

#[test]
fn passive_stdout_carries_only_json_rpc() {
    let (lines, stderr) = session(
        &["-vv", "mcp", "serve", "--stdio", "--passive"],
        &requests(),
    );
    assert_json_rpc_only(&lines);
    let tools: Value = serde_json::from_str(lines[1].trim_end_matches('\r')).unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bacnet_discover"]);
    assert!(stderr.contains("passive"), "logs go to stderr: {stderr}");
}

#[test]
fn a_bip_server_on_loopback_keeps_stdout_clean() {
    let (lines, stderr) = session(
        &[
            "-vv",
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "mcp",
            "serve",
            "--stdio",
        ],
        &requests(),
    );
    assert_json_rpc_only(&lines);
    let call: Value = serde_json::from_str(lines[2].trim_end_matches('\r')).unwrap();
    assert_eq!(call["result"]["isError"], json!(false), "{call}");
    assert!(
        stderr.contains("read call"),
        "the read summary is logged: {stderr}"
    );
}

#[test]
fn serve_needs_stdio() {
    let output = Command::new(env!("CARGO_BIN_EXE_bacnet"))
        .args(["mcp", "serve"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

/// A profile nobody authorized for MCP doesn't serve: the server refuses to
/// start, says how to authorize, and writes nothing to stdout.
#[test]
fn an_unauthorized_profile_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profiles")).unwrap();
    std::fs::write(
        dir.path().join("profiles/site.toml"),
        "version = 1\ntransport = \"bip\"\n\n[bip]\ninterface = \"127.0.0.1\"\nport = 0\n\n\
         [safety]\nwrites = \"never\"\ndangerous = false\n\n[mcp]\nnetworks = [0]\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bacnet"))
        .args(["mcp", "serve", "--stdio", "--profile", "site"])
        .env("BACNET_CONFIG_DIR", dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bacnet mcp authorize --profile site"),
        "{stderr}"
    );
}
