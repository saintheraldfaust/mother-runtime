use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn command(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motherctl"))
        .arg("--data-dir")
        .arg(dir)
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}

fn success(dir: &Path, args: &[&str]) -> Value {
    let result = command(dir, args);
    assert!(
        result.status.success(),
        "args={args:?}, stdout={}, stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn success_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_motherctl"));
    command.arg("--data-dir").arg(dir).arg("--json").args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "args={args:?}, stdout={}, stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

struct Runtime {
    child: Child,
    dir: PathBuf,
}
impl Runtime {
    fn start(dir: &Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_motherctl"))
            .arg("--data-dir")
            .arg(dir)
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut service = Self {
            child,
            dir: dir.to_owned(),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                service.child.try_wait().unwrap().is_none(),
                "service exited at startup"
            );
            if command(dir, &["status"]).status.success() {
                return service;
            }
            assert!(Instant::now() < deadline, "service startup timed out");
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn crash(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
    fn stop(&mut self) {
        assert_eq!(
            unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) },
            0
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "shutdown timed out");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!self.dir.join("runtime.sock").exists());
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("mother-")
        .tempdir_in("/tmp")
        .unwrap()
}

#[test]
fn independent_clients_concurrency_idempotency_and_crash_recovery() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    success(&dir, &["init"]);
    let mut runtime = Runtime::start(&dir);
    assert!(success(&dir, &["doctor"])["ok"].as_bool().unwrap());
    let repo = temp.path().to_str().unwrap();
    let project = success(
        &dir,
        &[
            "--request-id",
            "attach-1",
            "projects",
            "attach",
            "--name",
            "Example",
            "--repository",
            repo,
        ],
    );
    let id = project["result"]["id"].as_str().unwrap().to_owned();
    let replay = success(
        &dir,
        &[
            "--request-id",
            "attach-1",
            "projects",
            "attach",
            "--name",
            "Example",
            "--repository",
            repo,
        ],
    );
    assert_eq!(project, replay);
    let mut threads = Vec::new();
    for i in 0..8 {
        let dir = dir.clone();
        let id = id.clone();
        threads.push(thread::spawn(move || {
            success(
                &dir,
                &[
                    "--request-id",
                    &format!("write-{i}"),
                    "context",
                    "record",
                    "--project",
                    &id,
                    "--kind",
                    "observation",
                    "--source",
                    "client-a",
                    "--content",
                    &format!("finding-{i}"),
                ],
            )
        }));
    }
    for handle in threads {
        handle.join().unwrap();
    }
    let record = [
        "--request-id",
        "checkpoint-1",
        "context",
        "record",
        "--project",
        &id,
        "--kind",
        "blocker",
        "--content",
        "Contract unresolved",
        "--source",
        "client-a",
        "--expected-revision",
        "9",
    ];
    let first = success(&dir, &record);
    assert_eq!(first["result"]["revision"], 10);
    assert_eq!(success(&dir, &record), first); // Replay succeeds even though revision is now stale.
    let conflict = command(
        &dir,
        &[
            "--request-id",
            "checkpoint-1",
            "context",
            "record",
            "--project",
            &id,
            "--kind",
            "blocker",
            "--content",
            "Different payload",
            "--source",
            "client-a",
        ],
    );
    assert!(!conflict.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&conflict.stdout).unwrap()["error"]["code"],
        "conflict"
    );
    let stale = command(
        &dir,
        &[
            "context",
            "record",
            "--project",
            &id,
            "--kind",
            "fact",
            "--content",
            "stale write",
            "--source",
            "client-b",
            "--expected-revision",
            "1",
        ],
    );
    assert!(!stale.status.success());
    let before = success(&dir, &["context", "get", "--project", &id]);
    assert_eq!(before["result"]["revision"], 10);
    assert_eq!(before["result"]["items"].as_array().unwrap().len(), 9);
    runtime.crash();
    drop(runtime);
    let mut resumed = Runtime::start(&dir); // Also verifies safe stale-socket recovery.
    let after = success(&dir, &["context", "get", "--project", &id]);
    assert_eq!(before["result"], after["result"]);
    assert_eq!(success(&dir, &record), first); // Retry record survives process death.
    let page = success(&dir, &["context", "get", "--project", &id, "--limit", "4"]);
    let cursor = page["result"]["next_cursor"].as_u64().unwrap().to_string();
    let second = success(
        &dir,
        &["context", "get", "--project", &id, "--after", &cursor],
    );
    assert_eq!(second["result"]["items"].as_array().unwrap().len(), 5);
    let events = success(&dir, &["history", "--project", &id]);
    assert_eq!(events["result"]["items"].as_array().unwrap().len(), 10);
    assert!(success(&dir, &["doctor"])["ok"].as_bool().unwrap());
    resumed.stop();
}

#[test]
fn lock_private_paths_and_malformed_clients() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut runtime = Runtime::start(&dir);
    assert!(!command(&dir, &["serve"]).status.success());
    assert!(!command(&dir, &["init"]).status.success());
    for payload in [
        b"{bad json}\n".to_vec(),
        vec![b'x'; 65_537],
        b"{\"version\":99,\"request_id\":\"v\",\"operation\":{\"method\":\"health\"}}\n".to_vec(),
    ] {
        let mut stream = UnixStream::connect(dir.join("runtime.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(6)))
            .unwrap();
        stream.write_all(&payload).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let result: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(result["error"]["code"], "invalid_request");
    }
    success(&dir, &["status"]);
    assert_eq!(
        std::fs::metadata(dir.join("runtime.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    runtime.stop();
    std::fs::write(dir.join("runtime.sock"), "keep me").unwrap();
    assert!(!command(&dir, &["serve"]).status.success());
    assert_eq!(
        std::fs::read_to_string(dir.join("runtime.sock")).unwrap(),
        "keep me"
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!command(&dir, &["init"]).status.success());
}

#[test]
fn doctor_reports_missing_state_without_initializing() {
    let temp = directory();
    let dir = temp.path().join("absent");
    let result = command(&dir, &["doctor"]);
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["ok"],
        false
    );
    assert!(!dir.exists());
}

#[derive(Clone)]
struct McpClient {
    version: rmcp::model::ProtocolVersion,
}

impl rmcp::ClientHandler for McpClient {
    fn get_info(&self) -> rmcp::model::ClientConfig {
        rmcp::model::ClientConfig::default().with_protocol_version(self.version.clone())
    }
}

type ConnectedMcp = rmcp::service::RunningService<rmcp::RoleClient, McpClient>;

async fn mcp_client(dir: &Path, modern: bool) -> ConnectedMcp {
    use rmcp::{ClientServiceExt, model::ProtocolVersion, transport::TokioChildProcess};
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_motherctl"));
    command
        .arg("--data-dir")
        .arg(dir)
        .arg("mcp")
        .kill_on_drop(true);
    let client = McpClient {
        version: if modern {
            ProtocolVersion::V_2026_07_28
        } else {
            ProtocolVersion::V_2025_11_25
        },
    };
    let mode = if modern {
        rmcp::ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        }
    } else {
        rmcp::ClientLifecycleMode::Initialize
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        client.serve_with_lifecycle(TokioChildProcess::new(command).unwrap(), mode),
    )
    .await
    .unwrap()
    .unwrap()
}

async fn tool(client: &ConnectedMcp, name: &str, arguments: Value) -> rmcp::model::CallToolResult {
    tokio::time::timeout(
        Duration::from_secs(10),
        client.call_tool(
            rmcp::model::CallToolRequestParams::new(name.to_owned())
                .with_arguments(arguments.as_object().unwrap().clone()),
        ),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn mcp_clients_share_live_state_retries_resources_and_recover_after_restart() {
    tokio::time::timeout(Duration::from_secs(45), mcp_shared_state_scenario())
        .await
        .expect("MCP scenario timed out");
}

async fn mcp_shared_state_scenario() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut service = Runtime::start(&dir);
    let project = success(
        &dir,
        &[
            "projects",
            "attach",
            "--name",
            "MCP test",
            "--repository",
            temp.path().to_str().unwrap(),
        ],
    );
    let id = project["result"]["id"].as_str().unwrap();
    let a = mcp_client(&dir, false).await;
    let b = mcp_client(&dir, true).await;
    assert_eq!(
        a.peer_info().unwrap().protocol_version,
        rmcp::model::ProtocolVersion::V_2025_11_25
    );
    let tools = a.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 33);
    assert!(tools.iter().all(|t| t.output_schema.is_some()));
    let write_tool = tools
        .iter()
        .find(|t| t.name == "mother_context_record")
        .unwrap();
    assert_eq!(
        write_tool.annotations.as_ref().unwrap().read_only_hint,
        Some(false)
    );
    assert!(
        write_tool.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("idempotency_key"))
    );
    assert_eq!(
        tool(&a, "mother_projects_list", serde_json::json!({}))
            .await
            .structured_content
            .unwrap()["result"]["items"][0]["id"],
        id
    );
    let data = serde_json::json!({"idempotency_key":"mcp-checkpoint","project_id":id,"kind":"blocker","content":"Needs API contract","source":"client-a","expected_revision":1});
    let first = tool(&a, "mother_context_record", data.clone()).await;
    assert_eq!(first.is_error, Some(false));
    assert_eq!(
        tool(&b, "mother_context_record", data.clone())
            .await
            .structured_content,
        first.structured_content
    );
    let mut changed = data.clone();
    changed["content"] = serde_json::json!("different payload");
    let conflict = tool(&b, "mother_context_record", changed).await;
    assert_eq!(conflict.is_error, Some(true));
    assert_eq!(
        conflict.structured_content.unwrap()["error"]["code"],
        "conflict"
    );
    success(
        &dir,
        &[
            "context",
            "record",
            "--project",
            id,
            "--kind",
            "failure",
            "--content",
            "CLI test failed",
            "--source",
            "cli",
            "--expected-revision",
            "2",
        ],
    );
    for client in [&a, &b] {
        let context = tool(
            client,
            "mother_context_get",
            serde_json::json!({"project_id":id}),
        )
        .await
        .structured_content
        .unwrap();
        assert_eq!(context["result"]["revision"], 3);
        assert_eq!(context["result"]["items"].as_array().unwrap().len(), 2);
        let project = tool(
            client,
            "mother_project_get",
            serde_json::json!({"project_id":id}),
        )
        .await
        .structured_content
        .unwrap();
        assert_eq!(project["result"]["revision"], 3);
    }
    let activity = tool(
        &a,
        "mother_activity_get",
        serde_json::json!({"project_id":id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(activity["result"]["items"].as_array().unwrap().len(), 3);
    let first_page = tool(
        &b,
        "mother_context_get",
        serde_json::json!({"project_id":id,"limit":1}),
    )
    .await
    .structured_content
    .unwrap();
    let second_page = tool(
        &b,
        "mother_context_get",
        serde_json::json!({"project_id":id,"limit":1,"after":first_page["result"]["next_cursor"]}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(
        second_page["result"]["items"][0]["content"],
        "CLI test failed"
    );
    assert_eq!(b.list_all_resources().await.unwrap().len(), 1);
    assert_eq!(b.list_all_resource_templates().await.unwrap().len(), 7);
    let resource = b
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "mother://projects/{id}/context"
        )))
        .await
        .unwrap();
    let serialized = serde_json::to_value(resource).unwrap();
    let contents: Value =
        serde_json::from_str(serialized["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(contents["result"]["revision"], 3);
    assert!(
        b.read_resource(rmcp::model::ReadResourceRequestParams::new(
            "mother://projects/../../outside/context"
        ))
        .await
        .is_err()
    );
    let mut stale = data.clone();
    stale["idempotency_key"] = serde_json::json!("stale");
    assert_eq!(
        tool(&b, "mother_context_record", stale).await.is_error,
        Some(true)
    );
    let bad = tool(
        &a,
        "mother_context_get",
        serde_json::json!({"project_id":"missing"}),
    )
    .await;
    assert_eq!(bad.is_error, Some(true));
    // Required fields are enforced before dispatching a write.
    let invalid=a.call_tool(rmcp::model::CallToolRequestParams::new("mother_context_record").with_arguments(serde_json::json!({"project_id":id,"kind":"fact","content":"missing retry key","source":"test"}).as_object().unwrap().clone())).await;
    assert!(invalid.is_err() || invalid.unwrap().is_error == Some(true));
    assert!(
        a.call_tool(rmcp::model::CallToolRequestParams::new(
            "mother_execute_shell"
        ))
        .await
        .is_err()
    );
    service.crash();
    let offline = tool(&b, "mother_health", serde_json::json!({})).await;
    assert_eq!(offline.is_error, Some(true));
    assert_eq!(
        offline.structured_content.unwrap()["error"]["code"],
        "runtime_unavailable"
    );
    drop(service);
    let mut restarted = Runtime::start(&dir);
    assert_eq!(
        tool(&b, "mother_health", serde_json::json!({}))
            .await
            .is_error,
        Some(false)
    );
    assert_eq!(
        tool(&b, "mother_context_record", data)
            .await
            .structured_content,
        first.structured_content
    );
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
    let fresh = mcp_client(&dir, true).await;
    assert_eq!(
        tool(
            &fresh,
            "mother_project_get",
            serde_json::json!({"project_id":id})
        )
        .await
        .structured_content
        .unwrap()["result"]["revision"],
        3
    );
    fresh.cancel().await.unwrap();
    restarted.stop();
}

#[tokio::test]
async fn mcp_bridge_never_initializes_its_own_database() {
    let temp = directory();
    let dir = temp.path().join("absent");
    let client = mcp_client(&dir, true).await;
    let result = tool(&client, "mother_health", serde_json::json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(!dir.exists());
    client.cancel().await.unwrap();
}

#[test]
fn mcp_initialization_failure_never_prints_cli_json_on_stdout() {
    let temp = directory();
    let result = Command::new(env!("CARGO_BIN_EXE_motherctl"))
        .arg("--data-dir")
        .arg(temp.path().join("absent"))
        .arg("mcp")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!result.stderr.is_empty());
}

#[tokio::test]
async fn continuity_handoff_recovers_supersession_and_claims_once_across_clients() {
    tokio::time::timeout(Duration::from_secs(45), continuity_scenario())
        .await
        .expect("continuity scenario timed out");
}

async fn continuity_scenario() {
    use serde_json::json;
    use sha2::{Digest, Sha256};
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut service = Runtime::start(&dir);
    let project = success(
        &dir,
        &[
            "projects",
            "attach",
            "--name",
            "Continuity",
            "--repository",
            temp.path().to_str().unwrap(),
        ],
    );
    let id = project["result"]["id"].as_str().unwrap();
    let a = mcp_client(&dir, false).await;
    let b = mcp_client(&dir, true).await;
    let initial = success(
        &dir,
        &[
            "decisions",
            "record",
            "--project",
            id,
            "--title",
            "Database",
            "--decision",
            "Use MongoDB",
            "--rationale",
            "Initial proposal",
            "--source",
            "cli",
            "--expected-revision",
            "1",
        ],
    );
    let old = initial["result"]["id"].as_str().unwrap();
    let replacement = json!({"idempotency_key":"replacement","input":{"project_id":id,"title":"Database","decision":"Use SQLite","rationale":"Local-first runtime","alternatives":["MongoDB"],"source":"client-a","supersedes":old,"expected_revision":2}});
    let replaced = tool(&a, "mother_decision_record", replacement.clone()).await;
    assert_eq!(replaced.is_error, Some(false));
    let decision = replaced.structured_content.unwrap();
    assert_eq!(
        tool(&b, "mother_decision_record", replacement)
            .await
            .structured_content
            .unwrap(),
        decision
    );
    assert_eq!(
        tool(&b, "mother_decision_get", json!({"decision_id":old}))
            .await
            .structured_content
            .unwrap()["result"]["superseded_by"],
        decision["result"]["id"]
    );
    let active = tool(&b, "mother_decisions_list", json!({"project_id":id}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(active["result"]["items"].as_array().unwrap().len(), 1);
    let history = success(
        &dir,
        &["decisions", "list", "--project", id, "--include-superseded"],
    );
    assert_eq!(history["result"]["items"].as_array().unwrap().len(), 2);
    success(
        &dir,
        &[
            "context",
            "record",
            "--project",
            id,
            "--kind",
            "failure",
            "--content",
            "Authentication test failed: missing API contract",
            "--source",
            "client-a",
            "--expected-revision",
            "3",
        ],
    );
    let input = json!({"project_id":id,"objective":"Finish authentication","source":"client-a","originating_host":"Codex",
        "originating_session":"session-a","work_completed":["Drafted API types"],"work_remaining":["Implement validation"],
        "changed_files":["src/auth.rs"],"git_references":["local branch: auth"],"test_results":["authentication test failed"],
        "blockers":["API contract unresolved"],"unresolved_questions":["Which response format?"],"hypotheses":["Missing error mapping"],
        "provenance_references":["docs/api-notes.md"],"next_action":"Resolve the API contract and rerun authentication tests",
        "expected_recipient":"receiver","expected_revision":4});
    let file = temp.path().join("handoff.json");
    std::fs::write(&file, serde_json::to_vec_pretty(&input).unwrap()).unwrap();
    let created = success(
        &dir,
        &[
            "--request-id",
            "handoff-create",
            "handoff",
            "create",
            "--file",
            file.to_str().unwrap(),
        ],
    );
    let handoff = created["result"]["id"].as_str().unwrap();
    assert_eq!(created["result"]["snapshot_revision"], 4);
    assert_eq!(created["result"]["revision"], 5);
    assert_eq!(
        created["result"]["package"]["snapshot"]["active_decisions"][0]["id"],
        decision["result"]["id"]
    );
    assert_eq!(
        created["result"]["package"]["snapshot"]["context_records"][0]["kind"],
        "failure"
    );
    assert_eq!(
        created["result"]["package"]["verification"]["work_verified_by_runtime"],
        false
    );
    let expected_digest = Sha256::digest(
        created["result"]["package_json"]
            .as_str()
            .unwrap()
            .as_bytes(),
    );
    let digest_bytes: Vec<u8> = (0..32)
        .map(|i| {
            u8::from_str_radix(
                &created["result"]["sha256"].as_str().unwrap()[i * 2..i * 2 + 2],
                16,
            )
            .unwrap()
        })
        .collect();
    assert_eq!(expected_digest.as_slice(), digest_bytes.as_slice());
    a.cancel().await.unwrap();
    service.crash();
    drop(service);
    let mut restarted = Runtime::start(&dir);
    let recovered = tool(&b, "mother_handoff_get", json!({"handoff_id":handoff}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(recovered["result"], created["result"]);
    assert_eq!(
        recovered["result"]["package"]["submission"]["next_action"],
        input["next_action"]
    );
    let resource = b
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "mother://handoffs/{handoff}"
        )))
        .await
        .unwrap();
    let resource = serde_json::to_value(resource).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(resource["contents"][0]["text"].as_str().unwrap()).unwrap()["result"]
            ["sha256"],
        created["result"]["sha256"]
    );
    let bad_route=tool(&b,"mother_handoff_resume",json!({"idempotency_key":"wrong-recipient","input":{"handoff_id":handoff,"recipient":"other","expected_revision":5}})).await;
    assert_eq!(bad_route.is_error, Some(true));
    let stale_revision=tool(&b,"mother_handoff_resume",json!({"idempotency_key":"wrong-revision","input":{"handoff_id":handoff,"recipient":"receiver","expected_revision":4}})).await;
    assert_eq!(stale_revision.is_error, Some(true));
    let c = mcp_client(&dir, true).await;
    let claim_a = json!({"idempotency_key":"claim-a","input":{"handoff_id":handoff,"recipient":"receiver","expected_revision":5}});
    let claim_b = json!({"idempotency_key":"claim-b","input":{"handoff_id":handoff,"recipient":"receiver","expected_revision":5}});
    let (first, second) = tokio::join!(
        tool(&b, "mother_handoff_resume", claim_a.clone()),
        tool(&c, "mother_handoff_resume", claim_b.clone())
    );
    assert_ne!(first.is_error, second.is_error);
    let (winner, key) = if first.is_error == Some(false) {
        (first, claim_a)
    } else {
        (second, claim_b)
    };
    let claimed = winner.structured_content.unwrap();
    assert_eq!(claimed["result"]["claim"]["revision"], 6);
    assert_eq!(
        claimed["result"]["package_json"],
        created["result"]["package_json"]
    );
    restarted.crash();
    drop(restarted);
    let mut final_runtime = Runtime::start(&dir);
    let cli_replay = success(
        &dir,
        &[
            "--request-id",
            key["idempotency_key"].as_str().unwrap(),
            "handoff",
            "resume",
            "--handoff",
            handoff,
            "--recipient",
            "receiver",
            "--expected-revision",
            "5",
        ],
    );
    assert_eq!(cli_replay, claimed);
    assert_eq!(
        tool(&b, "mother_handoff_resume", key)
            .await
            .structured_content
            .unwrap(),
        claimed
    );
    assert_eq!(
        tool(&b, "mother_handoffs_list", json!({"project_id":id}))
            .await
            .structured_content
            .unwrap()["result"]["items"][0]["claimed_by"],
        "receiver"
    );
    let mut next = input.clone();
    next["expected_revision"] = json!(6);
    let next = tool(
        &b,
        "mother_handoff_create",
        json!({"idempotency_key":"second-handoff","input":next}),
    )
    .await;
    assert_eq!(next.is_error, Some(false));
    let next = next.structured_content.unwrap();
    let next_id = next["result"]["id"].as_str().unwrap();
    success(
        &dir,
        &[
            "context",
            "record",
            "--project",
            id,
            "--kind",
            "observation",
            "--content",
            "Project changed after checkpoint",
            "--source",
            "cli",
        ],
    );
    let stale=tool(&b,"mother_handoff_resume",json!({"idempotency_key":"stale-handoff","input":{"handoff_id":next_id,"recipient":"receiver","expected_revision":8}})).await;
    assert_eq!(stale.is_error, Some(true));
    let shown = success(&dir, &["handoff", "show", "--handoff", next_id]);
    assert!(shown["result"]["claim"].is_null());
    assert_eq!(shown["result"]["project_changed_since_creation"], true);
    assert_eq!(
        shown["result"]["package_json"],
        next["result"]["package_json"]
    );
    b.cancel().await.unwrap();
    c.cancel().await.unwrap();
    final_runtime.stop();
}

#[tokio::test]
async fn tasks_recover_checkpoints_and_complete_file_contracts_across_clients() {
    tokio::time::timeout(Duration::from_secs(45), task_scenario())
        .await
        .expect("task scenario timed out");
}

async fn task_scenario() {
    use serde_json::json;
    let temp = directory();
    let dir = temp.path().join("data");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    success(&dir, &["init"]);
    let mut service = Runtime::start(&dir);
    let project = success(
        &dir,
        &[
            "projects",
            "attach",
            "--name",
            "Task project",
            "--repository",
            repo.to_str().unwrap(),
        ],
    );
    let project_id = project["result"]["id"].as_str().unwrap();
    let contract = json!({"project_id":project_id,"title":"Create output","objective":"Create the contracted file","source":"cli",
        "acceptance_criteria":[{"path":"src/output.rs","expected_sha256":"b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"}]});
    let file = temp.path().join("task.json");
    std::fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    let task = success(
        &dir,
        &[
            "--request-id",
            "create-task",
            "tasks",
            "create",
            "--file",
            file.to_str().unwrap(),
        ],
    );
    let task_id = task["result"]["id"].as_str().unwrap();
    let a = mcp_client(&dir, false).await;
    let b = mcp_client(&dir, true).await;
    let mut child_input = contract.clone();
    child_input["title"] = json!("Dependent task");
    child_input["dependencies"] = json!([task_id]);
    let child = tool(
        &a,
        "mother_task_create",
        json!({"idempotency_key":"child-task","input":child_input}),
    )
    .await
    .structured_content
    .unwrap();
    let child_id = child["result"]["id"].as_str().unwrap();
    let triage = json!({"idempotency_key":"child-triage","input":{"task_id":child_id,"state":"triaged","source":"client-a","reason":"triage","expected_task_revision":1}});
    assert_eq!(
        tool(&a, "mother_task_transition", triage).await.is_error,
        Some(false)
    );
    let ready = json!({"idempotency_key":"child-ready","input":{"task_id":child_id,"state":"ready","source":"client-b","reason":"dependency complete","expected_task_revision":2}});
    assert_eq!(
        tool(&b, "mother_task_transition", ready.clone())
            .await
            .is_error,
        Some(true)
    );
    let mut task_revision = 1;
    for state in ["triaged", "ready", "assigned", "running"] {
        let mut args = vec![
            "tasks",
            "transition",
            "--task",
            task_id,
            "--state",
            state,
            "--source",
            "cli",
            "--reason",
            "host-tracked step",
            "--expected-task-revision",
        ];
        let revision = task_revision.to_string();
        args.push(&revision);
        if state == "assigned" {
            args.extend(["--owner-label", "engineer"]);
        }
        let step = success(&dir, &args);
        task_revision = step["result"]["task_revision"].as_u64().unwrap();
    }
    assert_eq!(task_revision, 5);
    let checkpoint = json!({"idempotency_key":"partial-checkpoint","input":{"task_id":task_id,"summary":"Partial implementation; waiting for artifact","source":"client-a",
        "work_remaining":["Create src/output.rs"],"blockers":["Contract not implemented"],"reported_test_results":["Local test failed: file missing"],"expected_task_revision":5}});
    let saved = tool(&a, "mother_task_checkpoint", checkpoint.clone()).await;
    assert_eq!(saved.is_error, Some(false));
    let saved = saved.structured_content.unwrap();
    assert_eq!(
        saved["result"]["latest_checkpoint"]["artifacts"][0]["status"],
        "missing"
    );
    let handoff=tool(&a,"mother_handoff_create",json!({"idempotency_key":"task-handoff","input":{"project_id":project_id,"task_id":task_id,
        "objective":"Continue the contracted task","source":"client-a","next_action":"Create file, validate, then checkpoint again"}})).await.structured_content.unwrap();
    let handoff_id = handoff["result"]["id"].as_str().unwrap();
    assert_eq!(
        handoff["result"]["package"]["snapshot"]["current_task"]["task_revision"],
        6
    );
    assert_eq!(
        handoff["result"]["package"]["snapshot"]["current_task"]["latest_checkpoint"]["submission"]
            ["blockers"][0],
        "Contract not implemented"
    );
    assert_eq!(
        handoff["result"]["package"]["snapshot"]["tasks"][0]["id"],
        child_id
    );
    a.cancel().await.unwrap();
    service.crash();
    drop(service);
    let mut restarted = Runtime::start(&dir);
    let recovered = tool(&b, "mother_task_get", json!({"task_id":task_id}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(recovered["result"]["task_revision"], 6);
    assert_eq!(recovered["result"]["state"], "running");
    assert_eq!(recovered["result"]["execution_mode"], "host_tracked");
    assert!(recovered["result"]["worker_execution"].is_null());
    assert_eq!(
        tool(&b, "mother_task_checkpoint", checkpoint)
            .await
            .structured_content
            .unwrap(),
        saved
    );
    let c = mcp_client(&dir, true).await;
    let validate = |key: &str| json!({"idempotency_key":key,"input":{"task_id":task_id,"state":"validating","source":"client-b","reason":"check artifact contract","expected_task_revision":6}});
    let (first, second) = tokio::join!(
        tool(&b, "mother_task_transition", validate("validate-a")),
        tool(&c, "mother_task_transition", validate("validate-b"))
    );
    assert_ne!(first.is_error, second.is_error);
    let loser = if first.is_error == Some(true) {
        first
    } else {
        second
    };
    assert_eq!(
        loser.structured_content.unwrap()["error"]["code"],
        "conflict"
    );
    std::fs::write(repo.join("src/output.rs"), b"hello world").unwrap();
    let checkpoint_file = temp.path().join("checkpoint.json");
    std::fs::write(&checkpoint_file,serde_json::to_vec(&json!({"task_id":task_id,"summary":"Contracted file created","source":"cli","reported_test_results":["No tests executed by Mother"],"expected_task_revision":7})).unwrap()).unwrap();
    let checkpoint = success(
        &dir,
        &[
            "tasks",
            "checkpoint",
            "--file",
            checkpoint_file.to_str().unwrap(),
        ],
    );
    assert_eq!(checkpoint["result"]["task_revision"], 8);
    assert_eq!(
        checkpoint["result"]["latest_checkpoint"]["artifacts"][0]["status"],
        "matched"
    );
    let complete = json!({"idempotency_key":"complete-task","input":{"task_id":task_id,"state":"completed","source":"client-b","reason":"file contract satisfied","expected_task_revision":8}});
    let done = tool(&b, "mother_task_transition", complete.clone()).await;
    assert_eq!(done.is_error, Some(false));
    let done = done.structured_content.unwrap();
    assert_eq!(done["result"]["state"], "completed");
    assert_eq!(
        done["result"]["completion_evidence"]["tests_verified"],
        false
    );
    assert_eq!(
        tool(&b, "mother_task_transition", ready).await.is_error,
        Some(false)
    );
    let checkpoints = tool(
        &b,
        "mother_task_checkpoints_list",
        json!({"task_id":task_id,"limit":1}),
    )
    .await
    .structured_content
    .unwrap();
    assert!(checkpoints["result"]["next_cursor"].is_number());
    let history = success(&dir, &["tasks", "history", "--task", task_id]);
    assert_eq!(history["result"]["items"].as_array().unwrap().len(), 6);
    let history = tool(
        &b,
        "mother_task_transitions_list",
        json!({"task_id":task_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(history["result"]["items"].as_array().unwrap().len(), 6);
    let list = success(
        &dir,
        &[
            "tasks",
            "list",
            "--project",
            project_id,
            "--state",
            "completed",
        ],
    );
    assert_eq!(list["result"]["items"][0]["id"], task_id);
    restarted.crash();
    drop(restarted);
    let mut final_runtime = Runtime::start(&dir);
    assert_eq!(
        tool(&b, "mother_task_transition", complete)
            .await
            .structured_content
            .unwrap(),
        done
    );
    let cli_replay = success(
        &dir,
        &[
            "--request-id",
            "complete-task",
            "tasks",
            "transition",
            "--task",
            task_id,
            "--state",
            "completed",
            "--source",
            "client-b",
            "--reason",
            "file contract satisfied",
            "--expected-task-revision",
            "8",
        ],
    );
    assert_eq!(cli_replay, done);
    let old = tool(&b, "mother_handoff_get", json!({"handoff_id":handoff_id}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(
        old["result"]["package_json"],
        handoff["result"]["package_json"]
    );
    assert_eq!(
        old["result"]["package"]["snapshot"]["current_task"]["state"],
        "running"
    );
    let resource = b
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "mother://tasks/{task_id}"
        )))
        .await
        .unwrap();
    let resource = serde_json::to_value(resource).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(resource["contents"][0]["text"].as_str().unwrap()).unwrap()["result"]
            ["state"],
        "completed"
    );
    let cancelled = success(
        &dir,
        &[
            "tasks",
            "transition",
            "--task",
            child_id,
            "--state",
            "cancelled",
            "--source",
            "cli",
            "--reason",
            "end host-tracked work",
            "--expected-task-revision",
            "3",
        ],
    );
    assert_eq!(cancelled["result"]["state"], "cancelled");
    assert!(success(&dir, &["doctor"])["ok"].as_bool().unwrap());
    b.cancel().await.unwrap();
    c.cancel().await.unwrap();
    final_runtime.stop();
}

#[tokio::test]
async fn persona_revisions_propagate_and_stale_capabilities_are_rejected() {
    tokio::time::timeout(Duration::from_secs(45), persona_live_policy_scenario())
        .await
        .expect("persona scenario timed out");
}

async fn persona_live_policy_scenario() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut service = Runtime::start(&dir);
    let project = success(
        &dir,
        &[
            "projects",
            "attach",
            "--name",
            "Persona test",
            "--repository",
            temp.path().to_str().unwrap(),
        ],
    );
    let project_id = project["result"]["id"].as_str().unwrap().to_owned();

    let persona_file = temp.path().join("persona.json");
    std::fs::write(
        &persona_file,
        serde_json::to_vec(&serde_json::json!({
            "name":"implementation-engineer",
            "role":"implementation_engineer",
            "description":"Implements bounded tasks and checkpoints",
            "responsibilities":["Implement reviewed changes"],
            "prohibitions":["Merge protected branches"],
            "capabilities":["task.create","task.transition"],
            "project_id":project_id,
            "preferred_runtime":"codex",
            "review_required":false,
            "max_concurrency":1
        }))
        .unwrap(),
    )
    .unwrap();
    let persona = success(
        &dir,
        &[
            "personas",
            "create",
            "--file",
            persona_file.to_str().unwrap(),
        ],
    );
    let persona_id = persona["result"]["id"].as_str().unwrap().to_owned();
    assert_eq!(persona["result"]["revision"], 1);

    let a = mcp_client(&dir, false).await;
    let b = mcp_client(&dir, true).await;
    let before = tool(
        &a,
        "mother_persona_get",
        serde_json::json!({"persona_id":persona_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(before["result"]["revision"], 1);
    assert!(
        before["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "task.transition")
    );

    let task = tool(
        &a,
        "mother_task_create",
        serde_json::json!({
            "idempotency_key":"persona-task",
            "input":{
                "project_id":project_id,
                "title":"Persona-gated task",
                "objective":"Verify current runtime policy",
                "source":"client-a",
                "acceptance_criteria":[{"path":"README.md"}],
                "persona":{"persona_id":persona_id,"expected_revision":1}
            }
        }),
    )
    .await
    .structured_content
    .unwrap();
    let task_id = task["result"]["id"].as_str().unwrap().to_owned();
    let triaged = tool(
        &a,
        "mother_task_transition",
        serde_json::json!({
            "idempotency_key":"persona-triaged",
            "input":{
                "task_id":task_id,
                "state":"triaged",
                "source":"client-a",
                "reason":"initial triage",
                "expected_task_revision":1,
                "persona":{"persona_id":persona_id,"expected_revision":1}
            }
        }),
    )
    .await;
    assert_eq!(triaged.is_error, Some(false));

    std::fs::write(
        &persona_file,
        serde_json::to_vec(&serde_json::json!({
            "name":"implementation-engineer",
            "role":"implementation_engineer",
            "description":"Revoked transition capability",
            "responsibilities":["Implement reviewed changes"],
            "prohibitions":["Merge protected branches"],
            "capabilities":["task.create"],
            "project_id":project_id,
            "preferred_runtime":"codex",
            "review_required":false,
            "max_concurrency":1
        }))
        .unwrap(),
    )
    .unwrap();
    let edited = success(
        &dir,
        &[
            "personas",
            "edit",
            "--persona",
            &persona_id,
            "--expected-revision",
            "1",
            "--file",
            persona_file.to_str().unwrap(),
        ],
    );
    assert_eq!(edited["result"]["revision"], 2);

    let refreshed = tool(
        &b,
        "mother_persona_get",
        serde_json::json!({"persona_id":persona_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(refreshed["result"]["revision"], 2);
    assert!(
        !refreshed["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "task.transition")
    );

    for expected_revision in [1, 2] {
        let denied = tool(
            &a,
            "mother_task_transition",
            serde_json::json!({
                "idempotency_key":format!("persona-ready-{expected_revision}"),
                "input":{
                    "task_id":task_id,
                    "state":"ready",
                    "source":"client-a",
                    "reason":"attempt stale policy",
                    "expected_task_revision":2,
                    "persona":{"persona_id":persona_id,"expected_revision":expected_revision}
                }
            }),
        )
        .await;
        assert_eq!(denied.is_error, Some(true));
        assert_eq!(
            denied.structured_content.unwrap()["error"]["code"],
            "conflict"
        );
    }

    let history = success(&dir, &["personas", "history", "--persona", &persona_id]);
    assert_eq!(history["result"]["items"].as_array().unwrap().len(), 2);
    a.cancel().await.ok();
    b.cancel().await.ok();
    service.stop();
}

fn git_repository(path: &Path) {
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.invalid"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(path.join("tracked.txt"), b"base\n").unwrap();
    run(&["add", "tracked.txt"]);
    run(&["commit", "-m", "base"]);
}

fn create_ready_task(dir: &Path, temp: &Path, project_id: &str, title: &str) -> String {
    let contract = serde_json::json!({"project_id":project_id,"title":title,"objective":"Exercise process supervision","source":"test",
        "acceptance_criteria":[{"path":"worker-output.txt"}]});
    let file = temp.join(format!("{}.json", title.replace(' ', "-").to_lowercase()));
    std::fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    let task = success(dir, &["tasks", "create", "--file", file.to_str().unwrap()]);
    let task_id = task["result"]["id"].as_str().unwrap().to_owned();
    let mut revision = 1;
    for state in ["triaged", "ready"] {
        let revision_arg = revision.to_string();
        let step = success(
            dir,
            &[
                "tasks",
                "transition",
                "--task",
                &task_id,
                "--state",
                state,
                "--source",
                "test",
                "--reason",
                "prepare",
                "--expected-task-revision",
                &revision_arg,
            ],
        );
        revision = step["result"]["task_revision"].as_u64().unwrap();
    }
    task_id
}

#[tokio::test]
async fn supervised_mock_process_uses_a_worktree_without_touching_the_project() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let temp = directory();
        let dir = temp.path().join("data");
        success(&dir, &["init"]);
        let mut runtime = Runtime::start(&dir);
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_repository(&repo);
        let project = success(
            &dir,
            &[
                "projects",
                "attach",
                "--name",
                "Process workers",
                "--repository",
                repo.to_str().unwrap(),
            ],
        );
        let project_id = project["result"]["id"].as_str().unwrap();
        let task_id = create_ready_task(&dir, temp.path(), project_id, "Supervised work");
        let run = success(
            &dir,
            &[
                "workers",
                "run",
                "--task",
                &task_id,
                "--worker-label",
                "process-a",
                "--lease-seconds",
                "5",
                "--max-active",
                "1",
                "--timeout-seconds",
                "10",
                "--helper-delay-ms",
                "600",
            ],
        );
        let execution = &run["result"];
        let execution_id = execution["id"].as_str().unwrap();
        assert_eq!(execution["status"], "released");
        assert_eq!(execution["process"]["status"], "exited");
        assert_eq!(execution["process"]["evidence"][0]["status"], "present");
        assert_eq!(execution["task_state"], "awaiting_input");
        assert_eq!(execution["repository_mutation"], false);
        assert_eq!(execution["worktree_mutation"], true);
        assert_eq!(execution["result"]["completion"], false);
        let worktree = PathBuf::from(execution["workspace"]["path"].as_str().unwrap());
        assert_eq!(
            std::fs::read(worktree.join("worker-output.txt")).unwrap(),
            format!("mock worker execution {execution_id}\n").as_bytes()
        );
        assert!(!repo.join("worker-output.txt").exists());
        let task = success(&dir, &["tasks", "inspect", "--task", &task_id]);
        assert_eq!(task["result"]["state"], "awaiting_input");
        assert_eq!(task["result"]["completion_evidence"], Value::Null);
        let log = execution["process"]["log_path"].as_str().unwrap();
        assert!(
            std::fs::read_to_string(log)
                .unwrap()
                .contains("worker-output.txt")
        );

        let cleaned = success(&dir, &["workers", "cleanup", "--execution", execution_id]);
        assert_eq!(cleaned["result"]["workspace"]["status"], "removed");
        assert!(!worktree.exists());
        let worktrees = Command::new("git")
            .current_dir(&repo)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .unwrap();
        assert!(worktrees.status.success());
        assert!(!String::from_utf8_lossy(&worktrees.stdout).contains(worktree.to_str().unwrap()));

        let second_id = create_ready_task(&dir, temp.path(), project_id, "Failed work");
        let failed = success(
            &dir,
            &[
                "workers",
                "run",
                "--task",
                &second_id,
                "--worker-label",
                "process-b",
                "--lease-seconds",
                "5",
                "--max-active",
                "1",
                "--timeout-seconds",
                "10",
                "--helper-delay-ms",
                "100",
                "--helper-exit-code",
                "2",
            ],
        );
        assert_eq!(failed["result"]["status"], "failed");
        assert_eq!(failed["result"]["process"]["status"], "failed");
        assert_eq!(failed["result"]["task_state"], "failed");
        assert_eq!(failed["result"]["result"]["completion"], false);
        let second_execution = failed["result"]["id"].as_str().unwrap();
        success(
            &dir,
            &["workers", "cleanup", "--execution", second_execution],
        );
        runtime.stop();
    })
    .await
    .expect("supervised process scenario timed out");
}

#[tokio::test]
async fn abandoned_supervisor_can_be_reconciled_and_cleaned_up() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let temp = directory();
        let dir = temp.path().join("data");
        success(&dir, &["init"]);
        let mut runtime = Runtime::start(&dir);
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_repository(&repo);
        let project = success(
            &dir,
            &[
                "projects",
                "attach",
                "--name",
                "Abandoned workers",
                "--repository",
                repo.to_str().unwrap(),
            ],
        );
        let project_id = project["result"]["id"].as_str().unwrap();
        let task_id = create_ready_task(&dir, temp.path(), project_id, "Crash work");
        let mut supervisor = Command::new(env!("CARGO_BIN_EXE_motherctl"))
            .arg("--data-dir")
            .arg(&dir)
            .arg("--json")
            .args([
                "workers",
                "run",
                "--task",
                &task_id,
                "--worker-label",
                "process-crash",
                "--lease-seconds",
                "30",
                "--max-active",
                "1",
                "--timeout-seconds",
                "60",
                "--helper-delay-ms",
                "60000",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let execution = loop {
            let listed = success(&dir, &["workers", "list", "--task", &task_id]);
            if let Some(value) = listed["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["process"]["status"] == "running")
            {
                break value.clone();
            }
            assert!(
                Instant::now() < deadline,
                "supervised process did not attach"
            );
            assert!(
                supervisor.try_wait().unwrap().is_none(),
                "supervised runner exited early"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let execution_id = execution["id"].as_str().unwrap();
        let worktree = PathBuf::from(execution["workspace"]["path"].as_str().unwrap());
        let refused = command(&dir, &["workers", "abandon", "--execution", execution_id]);
        assert!(!refused.status.success());
        assert!(
            String::from_utf8_lossy(&refused.stdout).contains("recorded process PID still exists")
        );
        supervisor.kill().unwrap();
        supervisor.wait().unwrap();
        thread::sleep(Duration::from_millis(500));
        let abandoned = success(&dir, &["workers", "abandon", "--execution", execution_id]);
        assert_eq!(abandoned["result"]["status"], "failed");
        assert_eq!(abandoned["result"]["process"]["status"], "lost");
        assert_eq!(abandoned["result"]["task_state"], "blocked");
        assert_eq!(abandoned["result"]["result"]["process_signal_sent"], false);
        let cleaned = success(&dir, &["workers", "cleanup", "--execution", execution_id]);
        assert_eq!(cleaned["result"]["workspace"]["status"], "removed");
        assert!(!worktree.exists());
        runtime.stop();
    })
    .await
    .expect("abandoned supervisor scenario timed out");
}

fn provider_script(temp: &Path, cost: f64, delay_seconds: u32, steps: u32) -> PathBuf {
    let path = temp.join("opencode");
    let script = format!(
        r#"#!/bin/sh
[ -d "$OPENCODE_CONFIG_DIR" ] || exit 10
[ "$OPENCODE_DISABLE_PROJECT_CONFIG" = "1" ] || exit 11
[ "$OPENCODE_DISABLE_AUTOUPDATE" = "1" ] || exit 12
[ "$HOME" != "$PWD" ] || exit 13
has_format=
has_pure=
has_auto=
has_model=
has_dir=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --standalone) exit 9 ;;
    --format) [ "$2" = "json" ] || exit 14; has_format=1; shift 2 ;;
    --pure) has_pure=1; shift ;;
    --auto) has_auto=1; shift ;;
    --model) [ "$2" = "test/model" ] || exit 15; has_model=1; shift 2 ;;
    --dir) [ -d "$2" ] || exit 16; has_dir=1; shift 2 ;;
    --title|--agent) shift 2 ;;
    *) shift ;;
  esac
done
[ "$has_format$has_pure$has_auto$has_model$has_dir" = "11111" ] || exit 17
printf '%s\n' '{{"type":"step_start","sessionID":"ses-test","part":{{"type":"step-start"}}}}'
printf 'provider output\n' > worker-output.txt
printf '%s\n' '{{"type":"tool_use","sessionID":"ses-test","part":{{"tool":"bash","state":{{"status":"completed"}}}}}}'
step=0
while [ "$step" -lt {steps} ]; do
  printf '%s\n' '{{"type":"step_finish","sessionID":"ses-test","part":{{"cost":{cost},"tokens":{{"input":100,"output":20,"reasoning":0,"cache":{{"read":0,"write":0}}}}}}}}'
  step=$((step + 1))
done
sleep {delay_seconds}
sleep 15 &
printf '%s\n' '{{"type":"text","sessionID":"ses-test","part":{{"text":"provider finished"}}}}'
"#
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[tokio::test]
async fn supervised_opencode_process_records_bounded_provider_telemetry() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let temp = directory();
        let dir = temp.path().join("data");
        success(&dir, &["init"]);
        let mut runtime = Runtime::start(&dir);
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_repository(&repo);
        let project = success(
            &dir,
            &[
                "projects",
                "attach",
                "--name",
                "Provider workers",
                "--repository",
                repo.to_str().unwrap(),
            ],
        );
        let project_id = project["result"]["id"].as_str().unwrap();
        let task_id = create_ready_task(&dir, temp.path(), project_id, "Provider work");
        let opencode = provider_script(temp.path(), 0.0125, 0, 1);
        let run = success_with_env(
            &dir,
            &[
                "workers",
                "run",
                "--task",
                &task_id,
                "--worker-label",
                "opencode-a",
                "--adapter",
                "opencode",
                "--model",
                "test/model",
                "--max-cost-usd",
                "1.0",
                "--max-steps",
                "5",
                "--lease-seconds",
                "5",
                "--max-active",
                "1",
                "--timeout-seconds",
                "10",
            ],
            &[("MOTHER_OPENCODE_BIN", opencode.to_str().unwrap())],
        );
        let execution = &run["result"];
        assert_eq!(execution["adapter"], "opencode");
        assert_eq!(execution["status"], "released");
        assert_eq!(execution["process"]["status"], "exited");
        assert_eq!(execution["process"]["evidence"][0]["status"], "present");
        assert_eq!(execution["task_state"], "awaiting_input");
        assert_eq!(execution["repository_mutation"], Value::Null);
        assert_eq!(execution["worktree_mutation"], true);
        let provider = &execution["result"]["provider"];
        assert_eq!(provider["session_id"], "ses-test");
        assert_eq!(provider["model"], "test/model");
        assert_eq!(provider["cost_usd"], 0.0125);
        assert_eq!(provider["tokens"]["input"], 100);
        assert_eq!(provider["tokens"]["output"], 20);
        assert_eq!(provider["steps"], 1);
        assert_eq!(provider["tool_calls"], 1);
        assert_eq!(provider["budget_exceeded"], false);
        assert!(
            provider["workspace_changes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|change| change["path"] == "worker-output.txt")
        );
        let worktree = PathBuf::from(execution["workspace"]["path"].as_str().unwrap());
        assert_eq!(
            std::fs::read_to_string(worktree.join("worker-output.txt")).unwrap(),
            "provider output\n"
        );
        assert!(!repo.join("worker-output.txt").exists());
        runtime.stop();
    })
    .await
    .expect("provider scenario timed out");
}

#[tokio::test]
async fn supervised_opencode_process_stops_when_cost_budget_is_exceeded() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let temp = directory();
        let dir = temp.path().join("data");
        success(&dir, &["init"]);
        let mut runtime = Runtime::start(&dir);
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_repository(&repo);
        let project = success(
            &dir,
            &[
                "projects",
                "attach",
                "--name",
                "Budget workers",
                "--repository",
                repo.to_str().unwrap(),
            ],
        );
        let project_id = project["result"]["id"].as_str().unwrap();
        let task_id = create_ready_task(&dir, temp.path(), project_id, "Budget work");
        let opencode = provider_script(temp.path(), 2.0, 5, 1);
        let run = success_with_env(
            &dir,
            &[
                "workers",
                "run",
                "--task",
                &task_id,
                "--worker-label",
                "opencode-budget",
                "--adapter",
                "opencode",
                "--model",
                "test/model",
                "--max-cost-usd",
                "0.5",
                "--max-steps",
                "5",
                "--lease-seconds",
                "5",
                "--max-active",
                "1",
                "--timeout-seconds",
                "10",
            ],
            &[("MOTHER_OPENCODE_BIN", opencode.to_str().unwrap())],
        );
        let execution = &run["result"];
        assert_eq!(execution["adapter"], "opencode");
        assert_eq!(execution["status"], "failed");
        assert_eq!(execution["process"]["status"], "killed");
        assert_eq!(execution["task_state"], "failed");
        assert_eq!(execution["result"]["provider"]["budget_exceeded"], true);
        assert_eq!(execution["result"]["provider"]["budget_reason"], "cost");
        assert_eq!(execution["result"]["provider"]["cost_usd"], 2.0);
        runtime.stop();
    })
    .await
    .expect("provider budget scenario timed out");
}

#[tokio::test]
async fn supervised_opencode_process_stops_when_step_budget_is_exceeded() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let temp = directory();
        let dir = temp.path().join("data");
        success(&dir, &["init"]);
        let mut runtime = Runtime::start(&dir);
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_repository(&repo);
        let project = success(
            &dir,
            &[
                "projects",
                "attach",
                "--name",
                "Step budget workers",
                "--repository",
                repo.to_str().unwrap(),
            ],
        );
        let project_id = project["result"]["id"].as_str().unwrap();
        let task_id = create_ready_task(&dir, temp.path(), project_id, "Step budget work");
        let opencode = provider_script(temp.path(), 0.01, 5, 3);
        let run = success_with_env(
            &dir,
            &[
                "workers",
                "run",
                "--task",
                &task_id,
                "--worker-label",
                "opencode-step-budget",
                "--adapter",
                "opencode",
                "--model",
                "test/model",
                "--max-cost-usd",
                "1.0",
                "--max-steps",
                "1",
                "--lease-seconds",
                "5",
                "--max-active",
                "1",
                "--timeout-seconds",
                "10",
            ],
            &[("MOTHER_OPENCODE_BIN", opencode.to_str().unwrap())],
        );
        let execution = &run["result"];
        assert_eq!(execution["status"], "failed");
        assert_eq!(execution["process"]["status"], "killed");
        assert_eq!(execution["task_state"], "failed");
        assert_eq!(execution["result"]["provider"]["budget_exceeded"], true);
        assert_eq!(execution["result"]["provider"]["budget_reason"], "steps");
        assert_eq!(execution["result"]["provider"]["steps"], 3);
        runtime.stop();
    })
    .await
    .expect("step budget scenario timed out");
}

#[tokio::test]
async fn supervised_opencode_process_rejects_instant_over_budget_completion() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let temp = directory();
        let dir = temp.path().join("data");
        success(&dir, &["init"]);
        let mut runtime = Runtime::start(&dir);
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git_repository(&repo);
        let project = success(
            &dir,
            &[
                "projects",
                "attach",
                "--name",
                "Instant budget workers",
                "--repository",
                repo.to_str().unwrap(),
            ],
        );
        let project_id = project["result"]["id"].as_str().unwrap();
        let task_id = create_ready_task(&dir, temp.path(), project_id, "Instant budget work");
        let opencode = provider_script(temp.path(), 3.0, 0, 1);
        let run = success_with_env(
            &dir,
            &[
                "workers",
                "run",
                "--task",
                &task_id,
                "--worker-label",
                "opencode-instant-budget",
                "--adapter",
                "opencode",
                "--model",
                "test/model",
                "--max-cost-usd",
                "0.5",
                "--max-steps",
                "5",
                "--lease-seconds",
                "5",
                "--max-active",
                "1",
                "--timeout-seconds",
                "10",
            ],
            &[("MOTHER_OPENCODE_BIN", opencode.to_str().unwrap())],
        );
        let execution = &run["result"];
        assert_eq!(execution["status"], "failed");
        assert_eq!(execution["process"]["status"], "killed");
        assert_eq!(execution["task_state"], "failed");
        assert_eq!(execution["result"]["provider"]["budget_exceeded"], true);
        assert_eq!(execution["result"]["provider"]["budget_reason"], "cost");
        runtime.stop();
    })
    .await
    .expect("instant provider budget scenario timed out");
}

#[tokio::test]
async fn worker_leases_schedule_checkpoint_and_recover_across_restart() {
    tokio::time::timeout(Duration::from_secs(45), worker_lease_scenario())
        .await
        .expect("worker scenario timed out");
}

async fn worker_lease_scenario() {
    use serde_json::json;
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut runtime = Runtime::start(&dir);
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let project = success(
        &dir,
        &[
            "projects",
            "attach",
            "--name",
            "Workers",
            "--repository",
            repo.to_str().unwrap(),
        ],
    );
    let project_id = project["result"]["id"].as_str().unwrap();
    let contract = json!({"project_id":project_id,"title":"Scheduled work","objective":"Exercise leases","source":"test",
        "acceptance_criteria":[{"path":"artifact"}]});
    let file = temp.path().join("worker-task.json");
    std::fs::write(&file, serde_json::to_vec(&contract).unwrap()).unwrap();
    let task = success(&dir, &["tasks", "create", "--file", file.to_str().unwrap()]);
    let task_id = task["result"]["id"].as_str().unwrap();
    let mut revision = 1;
    for state in ["triaged", "ready"] {
        let revision_arg = revision.to_string();
        let step = success(
            &dir,
            &[
                "tasks",
                "transition",
                "--task",
                task_id,
                "--state",
                state,
                "--source",
                "test",
                "--reason",
                "prepare",
                "--expected-task-revision",
                &revision_arg,
            ],
        );
        revision = step["result"]["task_revision"].as_u64().unwrap();
    }
    let eligible = success(&dir, &["workers", "eligible", "--project", project_id]);
    assert_eq!(eligible["result"]["count"], 1);
    let lease = success(
        &dir,
        &[
            "workers",
            "lease",
            "--task",
            task_id,
            "--worker-label",
            "mock-a",
            "--lease-seconds",
            "120",
            "--max-active",
            "1",
        ],
    );
    let execution_id = lease["result"]["id"].as_str().unwrap().to_owned();
    assert_eq!(lease["result"]["task_state"], "assigned");
    assert_eq!(lease["result"]["revision"], 1);
    let leased_task = success(&dir, &["tasks", "inspect", "--task", task_id]);
    assert_eq!(leased_task["result"]["execution_mode"], "worker_leased");
    assert_eq!(
        leased_task["result"]["worker_execution"]["id"],
        execution_id
    );
    let heartbeat = success(
        &dir,
        &[
            "workers",
            "heartbeat",
            "--execution",
            &execution_id,
            "--expected-revision",
            "1",
            "--lease-seconds",
            "60",
        ],
    );
    assert_eq!(heartbeat["result"]["revision"], 2);
    let mocked = success(
        &dir,
        &[
            "workers",
            "mock",
            "--execution",
            &execution_id,
            "--expected-revision",
            "2",
            "--summary",
            "Mock checkpoint only",
        ],
    );
    assert_eq!(mocked["result"]["execution"]["status"], "running");
    assert_eq!(mocked["result"]["execution"]["revision"], 3);
    assert_eq!(mocked["result"]["task"]["state"], "running");
    assert_eq!(mocked["result"]["limits"]["repository_mutation"], false);

    let second = success(&dir, &["tasks", "create", "--file", file.to_str().unwrap()]);
    let second_id = second["result"]["id"].as_str().unwrap();
    let mut second_revision = 1;
    for state in ["triaged", "ready"] {
        let revision_arg = second_revision.to_string();
        let step = success(
            &dir,
            &[
                "tasks",
                "transition",
                "--task",
                second_id,
                "--state",
                state,
                "--source",
                "test",
                "--reason",
                "prepare",
                "--expected-task-revision",
                &revision_arg,
            ],
        );
        second_revision = step["result"]["task_revision"].as_u64().unwrap();
    }
    let short = success(
        &dir,
        &[
            "workers",
            "lease",
            "--task",
            second_id,
            "--worker-label",
            "mock-b",
            "--lease-seconds",
            "1",
            "--max-active",
            "2",
        ],
    );
    assert_eq!(short["result"]["status"], "leased");
    thread::sleep(Duration::from_millis(1200));

    runtime.crash();
    drop(runtime);
    let mut restarted = Runtime::start(&dir);
    let recovered = success(&dir, &["workers", "recover"]);
    assert_eq!(recovered["result"]["count"], 1);
    assert_eq!(recovered["result"]["expired"][0]["status"], "expired");
    let requeued = success(&dir, &["tasks", "inspect", "--task", second_id]);
    assert_eq!(requeued["result"]["state"], "ready");
    assert!(requeued["result"]["worker_execution"].is_null());

    let mcp = mcp_client(&dir, false).await;
    let listed = tool(&mcp, "mother_worker_executions_list", json!({}))
        .await
        .structured_content
        .unwrap();
    assert!(
        listed["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == execution_id)
    );
    let inspected = tool(
        &mcp,
        "mother_worker_execution_get",
        json!({"execution_id":execution_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(inspected["result"]["revision"], 3);
    assert_eq!(inspected["result"]["process_supervised"], false);
    let history = tool(
        &mcp,
        "mother_worker_execution_history",
        json!({"execution_id":execution_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(history["result"]["items"].as_array().unwrap().len(), 3);
    let eligible = tool(&mcp, "mother_worker_eligible_tasks", json!({}))
        .await
        .structured_content
        .unwrap();
    assert!(
        eligible["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == second_id)
    );
    let resource = mcp
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "mother://worker-executions/{execution_id}"
        )))
        .await
        .unwrap();
    let serialized = serde_json::to_value(resource).unwrap();
    let contents: Value =
        serde_json::from_str(serialized["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(contents["result"]["id"], execution_id);
    mcp.cancel().await.unwrap();

    let released = success(
        &dir,
        &[
            "workers",
            "release",
            "--execution",
            &execution_id,
            "--expected-revision",
            "3",
            "--outcome",
            "failed",
            "--summary",
            "scenario finished",
        ],
    );
    assert_eq!(released["result"]["status"], "failed");
    restarted.stop();
}

#[tokio::test]
async fn github_identity_verification_is_read_only_and_revocable() {
    tokio::time::timeout(Duration::from_secs(45), github_identity_scenario())
        .await
        .expect("GitHub identity scenario timed out");
}

async fn github_identity_scenario() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut service = Runtime::start(&dir);

    let persona_file = temp.path().join("github-persona.json");
    std::fs::write(
        &persona_file,
        serde_json::to_vec(&serde_json::json!({
            "name":"account-reviewer",
            "role":"review_engineer",
            "description":"Reviews work through a read-only GitHub identity",
            "capabilities":["github.read"],
            "review_required":false,
            "max_concurrency":1
        }))
        .unwrap(),
    )
    .unwrap();
    let persona = success(
        &dir,
        &[
            "personas",
            "create",
            "--file",
            persona_file.to_str().unwrap(),
        ],
    );
    let persona_id = persona["result"]["id"].as_str().unwrap().to_owned();

    let profile = success(&dir, &["github", "profile", "--host", "github.com"]);
    let profile_id = profile["profile_id"].as_str().unwrap().to_owned();
    let config_dir = profile["config_dir"].as_str().unwrap();
    assert!(Path::new(config_dir).is_dir());

    let gh = temp.path().join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\n[ -z \"$GH_TOKEN\" ] || exit 7\n[ -z \"$GITHUB_TOKEN\" ] || exit 8\nprintf '%s' '{\"login\":\"alice\",\"id\":4242,\"name\":\"Alice Example\",\"type\":\"User\"}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();

    let connected = Command::new(env!("CARGO_BIN_EXE_motherctl"))
        .arg("--data-dir")
        .arg(&dir)
        .arg("--json")
        .args([
            "github",
            "connect",
            "--profile",
            &profile_id,
            "--host",
            "github.com",
            "--persona",
            &persona_id,
            "--expected-persona-revision",
            "1",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                temp.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("GH_TOKEN", "must-not-leak")
        .output()
        .unwrap();
    assert!(
        connected.status.success(),
        "stdout={}, stderr={}",
        String::from_utf8_lossy(&connected.stdout),
        String::from_utf8_lossy(&connected.stderr)
    );
    let identity: Value = serde_json::from_slice(&connected.stdout).unwrap();
    let identity_id = identity["result"]["id"].as_str().unwrap().to_owned();
    assert_eq!(identity["result"]["login"], "alice");
    assert_eq!(identity["result"]["account_id"], 4242);
    assert_eq!(identity["result"]["status"], "active");
    assert_eq!(
        identity["result"]["verification"]["credentials_stored"],
        false
    );
    assert!(!String::from_utf8_lossy(&connected.stdout).contains("must-not-leak"));

    let client = mcp_client(&dir, false).await;
    let listed = tool(
        &client,
        "mother_github_identities_list",
        serde_json::json!({"status":"active","persona_id":persona_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(listed["result"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"]["items"][0]["id"], identity_id);
    let fetched = tool(
        &client,
        "mother_github_identity_get",
        serde_json::json!({"identity_id":identity_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(fetched["result"]["persona_id"], persona_id);
    assert_eq!(fetched["result"]["current_persona_status"], "active");

    let revoked = success(
        &dir,
        &[
            "github",
            "revoke",
            "--identity",
            &identity_id,
            "--expected-revision",
            "1",
        ],
    );
    assert_eq!(revoked["result"]["status"], "revoked");
    assert_eq!(revoked["result"]["revision"], 2);
    let after = tool(
        &client,
        "mother_github_identity_get",
        serde_json::json!({"identity_id":identity_id}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(after["result"]["status"], "revoked");
    let active = tool(
        &client,
        "mother_github_identities_list",
        serde_json::json!({"status":"active"}),
    )
    .await
    .structured_content
    .unwrap();
    assert_eq!(active["result"]["items"].as_array().unwrap().len(), 0);
    let history = success(&dir, &["github", "history", "--identity", &identity_id]);
    assert_eq!(history["result"]["items"].as_array().unwrap().len(), 2);

    client.cancel().await.ok();
    service.stop();
}

#[test]
fn codex_supervisor_checks_flags_usage_failures_limits_and_private_output() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut runtime = Runtime::start(&dir);
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git_repository(&repo);
    let project = success(
        &dir,
        &[
            "projects",
            "attach",
            "--name",
            "Codex demo",
            "--repository",
            repo.to_str().unwrap(),
        ],
    );
    let project_id = project["result"]["id"].as_str().unwrap();
    for (name, tail, expected) in [
        (
            "Completed",
            r#"printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":3,"output_tokens":5}}'"#,
            "released",
        ),
        ("Missing completion", "true", "failed"),
        (
            "Failed turn",
            r#"printf '%s\n' '{"type":"turn.failed","error":{"message":"private diagnostic"}}'"#,
            "failed",
        ),
        (
            "Tool limit",
            r#"printf '%s\n' '{"type":"item.started","item":{"type":"command_execution"}}' '{"type":"item.started","item":{"type":"command_execution"}}' '{"type":"turn.completed","usage":{}}'"#,
            "failed",
        ),
        ("Timeout", "sleep 10", "failed"),
    ] {
        let task_id = create_ready_task(&dir, temp.path(), project_id, name);
        let script = temp.path().join("codex-helper");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
[ -z "$OPENAI_API_KEY$CODEX_API_KEY$GH_TOKEN$GITHUB_TOKEN" ] || exit 40
[ "$HOME" != "$PWD" ] || exit 41
[ "$1" = exec ] || exit 42
shift
ignore=; ephemeral=; json=; sandbox=; approvals=; provider=; mcp=; cwd=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --ignore-user-config) ignore=1; shift ;;
    --ephemeral) ephemeral=1; shift ;;
    --json) json=1; shift ;;
    --sandbox) [ "$2" = workspace-write ] || exit 43; sandbox=1; shift 2 ;;
    -c) case "$2" in
      'approval_policy="never"') approvals=1 ;;
      'model_provider="openai"') provider=1 ;;
      'mcp_servers={{}}') mcp=1 ;;
      *) exit 44 ;;
      esac; shift 2 ;;
    -C) [ "$2" -ef . ] || exit 45; cwd=1; shift 2 ;;
    --model) shift 2 ;;
    *) shift ;;
  esac
done
[ "$ignore$ephemeral$json$sandbox$approvals$provider$mcp$cwd" = 11111111 ] || exit 46
printf '%s\n' '{{"type":"thread.started","thread_id":"thread-demo"}}'
printf '%s\n' '{{"type":"item.completed","item":{{"type":"reasoning","text":"private reasoning"}}}}'
printf 'Codex artifact\n' > worker-output.txt
printf '%s\n' 'private stderr' >&2
{tail}
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let run = success_with_env(
            &dir,
            &[
                "workers",
                "run",
                "--adapter",
                "codex",
                "--task",
                &task_id,
                "--worker-label",
                "codex",
                "--timeout-seconds",
                if name == "Timeout" { "2" } else { "10" },
                "--max-steps",
                "1",
            ],
            &[
                ("MOTHER_CODEX_BIN", script.to_str().unwrap()),
                ("OPENAI_API_KEY", "test-env-must-be-removed"),
                ("GH_TOKEN", "test-env-must-be-removed"),
            ],
        );
        let worker = &run["result"];
        assert_eq!(worker["status"], expected, "scenario {name}: {worker}");
        assert_eq!(worker["result"]["provider"]["adapter"], "codex");
        assert_eq!(worker["result"]["provider"]["cost_usd"], Value::Null);
        assert_eq!(worker["result"]["completion"], false);
        assert!(!repo.join("worker-output.txt").exists());
        let log = std::fs::read_to_string(worker["process"]["log_path"].as_str().unwrap()).unwrap();
        assert!(!log.contains("private"));
        assert!(!worker.to_string().contains("private reasoning"));
        assert!(!worker.to_string().contains("private diagnostic"));
        assert!(!worker.to_string().contains("private stderr"));
        if name == "Completed" {
            assert_eq!(worker["task_state"], "awaiting_input");
            assert_eq!(worker["result"]["provider"]["tokens"]["input"], 10);
            assert_eq!(worker["process"]["evidence"][0]["status"], "present");
        }
        if name == "Tool limit" {
            assert_eq!(worker["result"]["provider"]["budget_reason"], "steps");
        }
        if name == "Timeout" {
            assert_eq!(worker["result"]["provider"]["budget_reason"], "timeout");
        }
    }
    let frame = command(&dir, &["ui", "--once"]);
    assert!(
        frame.status.success(),
        "{}",
        String::from_utf8_lossy(&frame.stderr)
    );
    let frame = String::from_utf8(frame.stdout).unwrap();
    assert!(frame.contains("Codex demo"));
    assert!(frame.contains("CONNECTED"));
    runtime.stop();
}

#[test]
fn codex_project_setup_preserves_configuration_and_rejects_conflicts() {
    let temp = directory();
    let dir = temp.path().join("data");
    success(&dir, &["init"]);
    let mut runtime = Runtime::start(&dir);
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let folder = repo.join(".codex");
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = folder.join("config.toml");
    std::fs::write(
        &file,
        "# Preserve this comment\nmodel = \"example-model\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let setup = || {
        Command::new(env!("CARGO_BIN_EXE_motherctl"))
            .arg("--data-dir")
            .arg(&dir)
            .arg("setup-codex")
            .current_dir(&repo)
            .output()
            .unwrap()
    };
    let first = setup();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let configured = std::fs::read_to_string(&file).unwrap();
    assert!(configured.starts_with("# Preserve this comment\nmodel = \"example-model\"\n"));
    assert!(configured.contains("[mcp_servers.mother]"));
    assert!(setup().status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), configured);
    assert_eq!(
        success(&dir, &["projects", "list"])["result"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let conflicting = configured.replace(env!("CARGO_BIN_EXE_motherctl"), "/other/motherctl");
    std::fs::write(&file, &conflicting).unwrap();
    assert!(!setup().status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), conflicting);
    runtime.stop();
}
