//! Prompt evaluator — systematically tests system prompt variations against real models
//! to find the best format compliance rate for <<<CREATE>>>/<<<EDIT>>>/<<<DELETE>>> blocks.
//!
//! Calls Ollama on csatapaci, parses responses with edit-parser, scores each prompt variant.
//! Iterates until a variant achieves >80% compliance across all test cases.

use std::fs;
use std::time::Instant;

use serde::{Deserialize, Serialize};

const OLLAMA_URL: &str = "http://100.81.10.8:11434";
const MODELS: &[&str] = &["deepseek-coder:33b", "qwen2.5-coder:7b"];
const MAX_ITERATIONS: usize = 10;
const TARGET_PASS_RATE: f64 = 0.80;
const RESULTS_DIR: &str = "eval/results";
const PROMPTS_DIR: &str = "eval/prompts";

/// Test case: a task + expected files + expected action type
struct TestCase {
    name: &'static str,
    task: &'static str,
    files: Vec<(&'static str, &'static str)>, // (path, content)
    expected_action: &'static str, // "create", "edit", "delete"
}

fn test_cases() -> Vec<TestCase> {
    vec![
        TestCase {
            name: "create_simple_file",
            task: "Create a file called hello.md with the text 'Hello World'",
            files: vec![],
            expected_action: "create",
        },
        TestCase {
            name: "create_readme",
            task: "Create README.md that describes this project as a Rust web server",
            files: vec![
                ("src/main.rs", "fn main() {\n    println!(\"Starting server on :8080\");\n}\n"),
            ],
            expected_action: "create",
        },
        TestCase {
            name: "edit_function",
            task: "Change the function greet to return 'Hi' instead of 'Hello'",
            files: vec![
                ("src/lib.rs", "pub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n"),
            ],
            expected_action: "edit",
        },
        TestCase {
            name: "edit_add_error_handling",
            task: "Add error handling to the parse function — return Result instead of panicking",
            files: vec![
                ("src/parser.rs", "pub fn parse(input: &str) -> i32 {\n    input.parse().unwrap()\n}\n"),
            ],
            expected_action: "edit",
        },
        TestCase {
            name: "create_test_file",
            task: "Create a test file tests/test_parser.rs with unit tests for the parse function",
            files: vec![
                ("src/parser.rs", "pub fn parse(input: &str) -> Result<i32, String> {\n    input.parse().map_err(|e| format!(\"parse error: {e}\"))\n}\n"),
            ],
            expected_action: "create",
        },
        TestCase {
            name: "delete_file",
            task: "Delete the file src/deprecated.rs",
            files: vec![
                ("src/deprecated.rs", "// This file is no longer used\npub fn old_function() {}\n"),
            ],
            expected_action: "delete",
        },
        TestCase {
            name: "create_with_context",
            task: "Create a CHANGELOG.md summarizing recent changes based on the git log shown in the context",
            files: vec![
                ("git_log.txt", "abc123 Add user auth\ndef456 Fix login bug\nghi789 Refactor database layer\n"),
            ],
            expected_action: "create",
        },
        TestCase {
            name: "edit_rename",
            task: "Rename the struct UserData to UserProfile everywhere in this file",
            files: vec![
                ("src/models.rs", "pub struct UserData {\n    pub name: String,\n    pub email: String,\n}\n\nimpl UserData {\n    pub fn new(name: String, email: String) -> UserData {\n        UserData { name, email }\n    }\n}\n"),
            ],
            expected_action: "edit",
        },
    ]
}

/// Prompt variants to test
fn prompt_variants() -> Vec<(String, String)> {
    vec![
        ("v1_original".into(), r#"You are a code modification agent. You receive a task description and file contents, then output precise file edits.

RULES:
- Only modify files that need to change
- Do not add unnecessary changes, comments, or formatting
- If the task is unclear or impossible, explain why instead of guessing

OUTPUT FORMAT:
For each file you want to modify, output a block like this:

<<<EDIT path/to/file.rs
--- OLD
the exact lines to replace (include enough context to be unique)
--- NEW
the replacement lines
>>>

For new files:

<<<CREATE path/to/new_file.rs
file contents here
>>>

For deleted files:

<<<DELETE path/to/file.rs
>>>

Output ONLY edit blocks. No explanation before or after unless the task cannot be done."#.into()),

        ("v2_emphasis".into(), r#"You are a code modification agent.

CRITICAL: Your response MUST contain ONLY edit blocks in the exact format shown below. Do NOT include any explanation, commentary, or markdown code fences. Start your response directly with <<< blocks.

FORMAT (use EXACTLY this syntax):

<<<CREATE path/to/file.ext
file content goes here
>>>

<<<EDIT path/to/file.ext
--- OLD
exact lines to find
--- NEW
replacement lines
>>>

<<<DELETE path/to/file.ext
>>>

RULES:
- Output NOTHING except <<< ... >>> blocks
- No markdown, no explanations, no code fences
- For new files, use <<<CREATE
- For modifications, use <<<EDIT with --- OLD and --- NEW markers
- For deletions, use <<<DELETE"#.into()),

        ("v3_examples".into(), r#"You are a code agent. Output ONLY edit blocks. No other text.

EXAMPLE 1 — Creating a file:
<<<CREATE src/hello.rs
fn main() {
    println!("Hello!");
}
>>>

EXAMPLE 2 — Editing a file:
<<<EDIT src/lib.rs
--- OLD
fn add(a: i32, b: i32) -> i32 {
    a + b
}
--- NEW
fn add(a: i32, b: i32) -> i64 {
    (a as i64) + (b as i64)
}
>>>

EXAMPLE 3 — Deleting a file:
<<<DELETE src/old.rs
>>>

Now complete the task. Output ONLY <<< blocks, nothing else."#.into()),

        ("v4_json_reminder".into(), r#"You are a code modification agent. You MUST respond using the structured edit format below. Any response not using this format will be rejected.

RESPONSE FORMAT — use these block types:

1. CREATE a new file:
<<<CREATE <filepath>
<entire file content>
>>>

2. EDIT an existing file:
<<<EDIT <filepath>
--- OLD
<exact existing lines>
--- NEW
<replacement lines>
>>>

3. DELETE a file:
<<<DELETE <filepath>
>>>

IMPORTANT:
- Start your response with <<< immediately
- Do not wrap in markdown code blocks (no ```)
- Do not explain what you're doing
- Every response must contain at least one <<< block"#.into()),

        ("v5_strict_minimal".into(), r#"Output edit blocks only. No text outside blocks.

<<<CREATE path
content
>>>

<<<EDIT path
--- OLD
old
--- NEW
new
>>>

<<<DELETE path
>>>

Start with <<<. No explanations."#.into()),
    ]
}

#[derive(Serialize, Deserialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: Option<OllamaMessageResp>,
    #[serde(default)]
    eval_count: u32,
    #[serde(default)]
    prompt_eval_count: u32,
}

#[derive(Deserialize)]
struct OllamaMessageResp {
    content: String,
}

struct EvalResult {
    test_name: String,
    model: String,
    prompt_variant: String,
    response: String,
    parsed_ok: bool,
    edit_count: usize,
    correct_action: bool,
    tokens_out: u32,
    duration_ms: u64,
}

fn call_ollama(model: &str, system: &str, user: &str) -> Result<(String, u32, u64), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("client: {e}"))?;

    let request = OllamaRequest {
        model: model.into(),
        messages: vec![
            OllamaMessage { role: "system".into(), content: system.into() },
            OllamaMessage { role: "user".into(), content: user.into() },
        ],
        stream: false,
    };

    let start = Instant::now();
    let resp = client.post(format!("{OLLAMA_URL}/api/chat"))
        .json(&request)
        .send()
        .map_err(|e| format!("send: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body: OllamaResponse = resp.json().map_err(|e| format!("json: {e}"))?;
    let content = body.message.map(|m| m.content).unwrap_or_default();
    let duration = start.elapsed().as_millis() as u64;

    Ok((content, body.eval_count, duration))
}

fn build_user_message(tc: &TestCase) -> String {
    let mut msg = format!("TASK: {}\n\n", tc.task);
    if !tc.files.is_empty() {
        msg.push_str("FILES:\n");
        for (path, content) in &tc.files {
            if content.is_empty() {
                msg.push_str(&format!("=== {path} === [NEW FILE — does not exist yet, use <<<CREATE>>> to create it]\n\n"));
            } else {
                msg.push_str(&format!("=== {path} ===\n{content}\n\n"));
            }
        }
    }
    msg
}

fn evaluate(tc: &TestCase, model: &str, prompt_name: &str, system_prompt: &str) -> EvalResult {
    let user_msg = build_user_message(tc);

    let (response, tokens_out, duration_ms) = match call_ollama(model, system_prompt, &user_msg) {
        Ok(r) => r,
        Err(e) => {
            return EvalResult {
                test_name: tc.name.into(), model: model.into(), prompt_variant: prompt_name.into(),
                response: format!("ERROR: {e}"), parsed_ok: false, edit_count: 0,
                correct_action: false, tokens_out: 0, duration_ms: 0,
            };
        }
    };

    let edits = edit_parser::parse_edits(&response).unwrap_or_default();
    let parsed_ok = !edits.is_empty();

    let correct_action = match tc.expected_action {
        "create" => edits.iter().any(|e| matches!(e, edit_parser::FileEdit::Create { .. })),
        "edit" => edits.iter().any(|e| matches!(e, edit_parser::FileEdit::Edit { .. })),
        "delete" => edits.iter().any(|e| matches!(e, edit_parser::FileEdit::Delete { .. })),
        _ => false,
    };

    EvalResult {
        test_name: tc.name.into(), model: model.into(), prompt_variant: prompt_name.into(),
        response, parsed_ok, edit_count: edits.len(), correct_action, tokens_out, duration_ms,
    }
}

fn write_iteration_report(iteration: usize, results: &[EvalResult], prompt_name: &str, system_prompt: &str) {
    let total = results.len();
    let parsed = results.iter().filter(|r| r.parsed_ok).count();
    let correct = results.iter().filter(|r| r.correct_action).count();
    let pass_rate = if total > 0 { parsed as f64 / total as f64 } else { 0.0 };
    let correct_rate = if total > 0 { correct as f64 / total as f64 } else { 0.0 };

    let mut report = format!(
        "# Prompt Evaluation — Iteration {iteration}\n\n\
         **Prompt variant**: {prompt_name}\n\
         **Date**: {}\n\
         **Parse rate**: {parsed}/{total} ({:.0}%)\n\
         **Correct action rate**: {correct}/{total} ({:.0}%)\n\n\
         ## System Prompt\n\n```\n{system_prompt}\n```\n\n\
         ## Results\n\n\
         | Test | Model | Parsed | Correct | Edits | Tokens | Time |\n\
         |------|-------|--------|---------|-------|--------|------|\n",
        chrono::Utc::now().to_rfc3339(),
        pass_rate * 100.0,
        correct_rate * 100.0,
    );

    for r in results {
        let parsed_icon = if r.parsed_ok { "pass" } else { "FAIL" };
        let correct_icon = if r.correct_action { "pass" } else { "FAIL" };
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {}ms |\n",
            r.test_name, r.model, parsed_icon, correct_icon, r.edit_count, r.tokens_out, r.duration_ms,
        ));
    }

    // Add response samples for failures
    let failures: Vec<&EvalResult> = results.iter().filter(|r| !r.parsed_ok).collect();
    if !failures.is_empty() {
        report.push_str("\n## Failed Responses (no edit blocks parsed)\n\n");
        for f in failures.iter().take(3) {
            let preview = if f.response.len() > 500 { &f.response[..500] } else { &f.response };
            report.push_str(&format!("### {} ({})\n```\n{}\n```\n\n", f.test_name, f.model, preview));
        }
    }

    let path = format!("{RESULTS_DIR}/iteration_{iteration}.md");
    fs::write(&path, &report).expect("write report");
    println!("  Wrote {path}");
}

fn write_prompt_file(iteration: usize, name: &str, prompt: &str) {
    let path = format!("{PROMPTS_DIR}/{name}.txt");
    fs::write(&path, prompt).expect("write prompt");
}

fn main() {
    println!("=== Prompt Evaluator ===\n");

    fs::create_dir_all(RESULTS_DIR).ok();
    fs::create_dir_all(PROMPTS_DIR).ok();

    let cases = test_cases();
    let variants = prompt_variants();

    let mut best_rate = 0.0f64;
    let mut best_variant = String::new();

    for (iteration, (name, system_prompt)) in variants.iter().enumerate() {
        let iteration_num = iteration + 1;
        println!("--- Iteration {iteration_num}: {name} ---");
        write_prompt_file(iteration_num, name, system_prompt);

        let mut results = Vec::new();

        for model in MODELS {
            println!("  Model: {model}");
            for tc in &cases {
                print!("    {}: ", tc.name);
                let result = evaluate(tc, model, name, system_prompt);
                let icon = if result.parsed_ok { "OK" } else { "FAIL" };
                println!("{icon} (edits={}, tok={}, {}ms)", result.edit_count, result.tokens_out, result.duration_ms);
                results.push(result);
            }
        }

        let total = results.len();
        let parsed = results.iter().filter(|r| r.parsed_ok).count();
        let rate = if total > 0 { parsed as f64 / total as f64 } else { 0.0 };

        write_iteration_report(iteration_num, &results, name, system_prompt);

        println!("  Parse rate: {parsed}/{total} ({:.0}%)\n", rate * 100.0);

        if rate > best_rate {
            best_rate = rate;
            best_variant = name.clone();
        }

        if rate >= TARGET_PASS_RATE {
            println!("TARGET REACHED! Variant '{name}' achieves {:.0}% compliance.", rate * 100.0);
            break;
        }
    }

    // Write summary
    let summary = format!(
        "# Prompt Evaluation Summary\n\n\
         **Best variant**: {best_variant}\n\
         **Best parse rate**: {:.0}%\n\
         **Target**: {:.0}%\n\
         **Models tested**: {}\n\
         **Test cases**: {}\n\
         **Iterations run**: {}\n",
        best_rate * 100.0,
        TARGET_PASS_RATE * 100.0,
        MODELS.join(", "),
        test_cases().len(),
        variants.len().min(MAX_ITERATIONS),
    );
    fs::write(format!("{RESULTS_DIR}/SUMMARY.md"), &summary).expect("write summary");
    println!("\nSummary written to {RESULTS_DIR}/SUMMARY.md");
    println!("Best: {best_variant} at {:.0}%", best_rate * 100.0);
}
