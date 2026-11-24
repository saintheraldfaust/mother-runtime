//! Bounded telemetry from the owner-run OpenCode CLI adapter.
//!
//! OpenCode emits newline-delimited JSON with `run --format json`. Mother keeps
//! only provider-reported accounting metadata and bounded errors; it never stores
//! hidden reasoning or treats model text as completion evidence.
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

const LOG_LIMIT: usize = 64 * 1024;
const LINE_LIMIT: usize = 256 * 1024;
const ERROR_LIMIT: usize = 512;
const MAX_ERRORS: usize = 5;
const MAX_CHANGE_BYTES: usize = 12 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Telemetry {
    pub session_id: Option<String>,
    pub cost_usd: f64,
    pub tokens: Tokens,
    pub steps: u64,
    pub tool_calls: u64,
    pub retries: u64,
    pub errors: Vec<String>,
    pub parse_errors: u64,
    pub lines: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Telemetry {
    pub fn observe(&mut self, line: &[u8]) {
        self.lines += 1;
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            self.parse_errors += 1;
            return;
        };
        if let Some(session) = value.get("sessionID").and_then(Value::as_str)
            && !session.is_empty()
            && session.len() <= 128
        {
            self.session_id = Some(session.to_owned());
        }
        match value.get("type").and_then(Value::as_str) {
            Some("step_finish") => {
                self.steps += 1;
                if let Some(cost) = value
                    .get("part")
                    .and_then(|part| part.get("cost"))
                    .and_then(Value::as_f64)
                    && cost.is_finite()
                    && cost >= 0.0
                {
                    self.cost_usd += cost;
                }
                if let Some(tokens) = value.get("part").and_then(|part| part.get("tokens")) {
                    let input = tokens.get("input").and_then(Value::as_u64).unwrap_or(0);
                    let output = tokens.get("output").and_then(Value::as_u64).unwrap_or(0);
                    let reasoning = tokens.get("reasoning").and_then(Value::as_u64).unwrap_or(0);
                    let cache = tokens.get("cache");
                    self.tokens.input = self.tokens.input.saturating_add(input);
                    self.tokens.output = self.tokens.output.saturating_add(output);
                    self.tokens.reasoning = self.tokens.reasoning.saturating_add(reasoning);
                    self.tokens.cache_read = self.tokens.cache_read.saturating_add(
                        cache
                            .and_then(|cache| cache.get("read"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    );
                    self.tokens.cache_write = self.tokens.cache_write.saturating_add(
                        cache
                            .and_then(|cache| cache.get("write"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    );
                }
            }
            Some("tool_use") => self.tool_calls += 1,
            Some("retry") => self.retries += 1,
            Some("error") if self.errors.len() < MAX_ERRORS => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .or_else(|| value.get("error").and_then(Value::as_str))
                    .unwrap_or("provider error");
                self.errors.push(bounded(message, ERROR_LIMIT));
            }
            _ => {}
        }
    }

    pub fn metadata(
        &self,
        model: &str,
        max_cost_usd: f64,
        max_steps: u64,
        timeout_seconds: u64,
        budget_reason: Option<&str>,
        mut workspace_changes: Vec<Value>,
    ) -> Value {
        while !workspace_changes.is_empty()
            && serde_json::to_vec(&workspace_changes)
                .map_or(true, |value| value.len() > MAX_CHANGE_BYTES)
        {
            workspace_changes.pop();
        }
        json!({
            "adapter":"opencode",
            "session_id":self.session_id,
            "model":model,
            "cost_usd":self.cost_usd,
            "tokens":self.tokens,
            "steps":self.steps,
            "tool_calls":self.tool_calls,
            "retries":self.retries,
            "errors":self.errors,
            "parse_errors":self.parse_errors,
            "lines":self.lines,
            "budget":{
                "max_cost_usd":max_cost_usd,
                "max_steps":max_steps,
                "timeout_seconds":timeout_seconds
            },
            "budget_exceeded":budget_reason.is_some(),
            "budget_reason":budget_reason,
            "workspace_changes":workspace_changes,
            "worktree_mutation":!workspace_changes.is_empty()
        })
    }
}

fn bounded(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Capture bounded raw output while parsing complete JSON lines into telemetry.
pub fn monitor(
    mut stream: impl Read + Send + 'static,
    telemetry: Arc<Mutex<Telemetry>>,
) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut pending = Vec::new();
        let mut discarding = false;
        let mut chunk = [0u8; 8192];
        loop {
            let read = match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let room = LOG_LIMIT.saturating_sub(captured.len());
            captured.extend_from_slice(&chunk[..read.min(room)]);
            for byte in &chunk[..read] {
                if *byte == b'\n' {
                    if !discarding
                        && !pending.is_empty()
                        && let Ok(mut telemetry) = telemetry.lock()
                    {
                        telemetry.observe(&pending);
                    }
                    pending.clear();
                    discarding = false;
                } else if !discarding {
                    if pending.len() < LINE_LIMIT {
                        pending.push(*byte);
                    } else {
                        if let Ok(mut telemetry) = telemetry.lock() {
                            telemetry.parse_errors += 1;
                        }
                        pending.clear();
                        discarding = true;
                    }
                }
            }
        }
        if !discarding
            && !pending.is_empty()
            && let Ok(mut telemetry) = telemetry.lock()
        {
            telemetry.observe(&pending);
        }
        captured
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_accounting_without_storing_text_or_reasoning() {
        let lines = concat!(
            "{\"type\":\"step_start\",\"sessionID\":\"ses_demo\",\"part\":{}}\n",
            "{\"type\":\"text\",\"sessionID\":\"ses_demo\",\"part\":{\"text\":\"secret answer\"}}\n",
            "{\"type\":\"tool_use\",\"sessionID\":\"ses_demo\",\"part\":{\"tool\":\"bash\"}}\n",
            "{\"type\":\"step_finish\",\"sessionID\":\"ses_demo\",\"part\":{\"cost\":0.02,\"tokens\":{\"input\":10,\"output\":4,\"reasoning\":2,\"cache\":{\"read\":3,\"write\":1}}}}\n",
            "not-json\n"
        );
        let telemetry = Arc::new(Mutex::new(Telemetry::default()));
        let captured = monitor(Cursor::new(lines.as_bytes()), telemetry.clone())
            .join()
            .unwrap();
        let telemetry = telemetry.lock().unwrap();
        assert_eq!(telemetry.session_id.as_deref(), Some("ses_demo"));
        assert_eq!(telemetry.cost_usd, 0.02);
        assert_eq!(telemetry.steps, 1);
        assert_eq!(telemetry.tool_calls, 1);
        assert_eq!(telemetry.tokens.input, 10);
        assert_eq!(telemetry.tokens.cache_write, 1);
        assert_eq!(telemetry.parse_errors, 1);
        assert!(String::from_utf8_lossy(&captured).contains("secret answer"));
        let metadata = telemetry.metadata("test/model", 1.0, 2, 30, None, vec![]);
        assert!(metadata.get("text").is_none());
        assert!(metadata.get("reasoning").is_none());
        assert_eq!(metadata["session_id"], "ses_demo");
    }

    #[test]
    fn workspace_changes_are_trimmed_before_metadata_size_limit() {
        let changes = (0..64)
            .map(|_| {
                json!({
                    "status":"??",
                    "path": "\u{1}".repeat(256)
                })
            })
            .collect();
        let metadata = Telemetry::default().metadata("test/model", 1.0, 2, 30, None, changes);
        let encoded = serde_json::to_vec(&metadata).unwrap();
        assert!(encoded.len() < 32 * 1024);
        assert!(!metadata["workspace_changes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn oversized_lines_are_discarded_without_unbounded_growth() {
        let mut line = vec![b'x'; LINE_LIMIT + 10];
        line.push(b'\n');
        let telemetry = Arc::new(Mutex::new(Telemetry::default()));
        monitor(Cursor::new(line), telemetry.clone())
            .join()
            .unwrap();
        assert_eq!(telemetry.lock().unwrap().parse_errors, 1);
    }
}
