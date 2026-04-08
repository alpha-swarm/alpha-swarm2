use async_trait::async_trait;
use serde_json::{json, Value};
use crate::{Tool, ToolContext, ToolResult};

/// Max characters to return from a web fetch
const MAX_FETCH_CHARS: usize = 10_000;
/// Max search results to return
const MAX_SEARCH_RESULTS: usize = 5;

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str { "Search the web using DuckDuckGo (no API key needed)" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("");
        if query.is_empty() {
            return ToolResult::err("Missing 'query' parameter", 0);
        }

        // Use DuckDuckGo HTML lite — no API key needed
        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoded(query));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (compatible; alpha-swarm/0.1)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        match client.get(&url).send().await {
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                let results = parse_ddg_results(&body);
                if results.is_empty() {
                    ToolResult::ok(format!("No results found for '{query}'"), 0)
                } else {
                    let formatted: Vec<String> = results.iter().take(MAX_SEARCH_RESULTS)
                        .map(|(title, snippet, url)| format!("- {} ({})\n  {}", title, url, snippet))
                        .collect();
                    ToolResult::ok(formatted.join("\n\n"), 0)
                }
            }
            Err(e) => ToolResult::err(format!("Search failed: {e}"), 0),
        }
    }
}

pub struct FetchUrlTool;

#[async_trait]
impl Tool for FetchUrlTool {
    fn name(&self) -> &str { "fetch_url" }
    fn description(&self) -> &str { "Fetch a URL and extract its text content" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to fetch"}
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let url = params.get("url").and_then(|u| u.as_str()).unwrap_or("");
        if url.is_empty() {
            return ToolResult::err("Missing 'url' parameter", 0);
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (compatible; alpha-swarm/0.1)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        match client.get(url).send().await {
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                // Strip HTML tags for a rough text extraction
                let text = strip_html(&body);
                if text.len() > MAX_FETCH_CHARS {
                    ToolResult::ok(format!("{}...\n(truncated, {} chars total)", &text[..MAX_FETCH_CHARS], text.len()), 0)
                } else {
                    ToolResult::ok(text, 0)
                }
            }
            Err(e) => ToolResult::err(format!("Fetch failed: {e}"), 0),
        }
    }
}

pub struct SearchCratesTool;

#[async_trait]
impl Tool for SearchCratesTool {
    fn name(&self) -> &str { "search_crates" }
    fn description(&self) -> &str { "Search crates.io for Rust crates" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query for crates.io"}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("");
        if query.is_empty() {
            return ToolResult::err("Missing 'query' parameter", 0);
        }

        let url = format!("https://crates.io/api/v1/crates?q={}&per_page={}", urlencoded(query), MAX_SEARCH_RESULTS);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("alpha-swarm/0.1 (https://github.com/alpha-swarm)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        match client.get(&url).send().await {
            Ok(resp) => {
                let body: Value = resp.json().await.unwrap_or_default();
                let crates = body.get("crates").and_then(|c| c.as_array());
                match crates {
                    Some(arr) => {
                        let results: Vec<String> = arr.iter().map(|c| {
                            let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                            let desc = c.get("description").and_then(|d| d.as_str()).unwrap_or("");
                            let dl = c.get("downloads").and_then(|d| d.as_u64()).unwrap_or(0);
                            let ver = c.get("newest_version").and_then(|v| v.as_str()).unwrap_or("?");
                            format!("- {} v{} ({} downloads)\n  {}", name, ver, dl, desc)
                        }).collect();
                        ToolResult::ok(results.join("\n"), 0)
                    }
                    None => ToolResult::ok(format!("No crates found for '{query}'"), 0),
                }
            }
            Err(e) => ToolResult::err(format!("crates.io search failed: {e}"), 0),
        }
    }
}

/// Simple URL encoding for query parameters
fn urlencoded(s: &str) -> String {
    s.chars().map(|c| match c {
        ' ' => '+'.to_string(),
        c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' => c.to_string(),
        c => format!("%{:02X}", c as u32),
    }).collect()
}

/// Strip HTML tags for rough text extraction
fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            // Check for script/style tags
            let rest = &html[html.len().min(result.len())..];
            if rest.starts_with("<script") || rest.starts_with("<style") {
                in_script = true;
            }
            if rest.starts_with("</script") || rest.starts_with("</style") {
                in_script = false;
            }
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag && !in_script {
            result.push(c);
        }
    }
    // Collapse whitespace
    let mut collapsed = String::new();
    let mut last_ws = false;
    for c in result.chars() {
        if c.is_whitespace() {
            if !last_ws { collapsed.push(' '); }
            last_ws = true;
        } else {
            collapsed.push(c);
            last_ws = false;
        }
    }
    collapsed.trim().to_string()
}

/// Parse DuckDuckGo HTML lite results (fragile but works without API key)
fn parse_ddg_results(html: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    // DDG lite wraps results in <a class="result__a"> with <a class="result__snippet">
    for chunk in html.split("result__a") {
        if results.len() >= MAX_SEARCH_RESULTS { break; }
        // Extract href
        let url = extract_attr(chunk, "href=\"");
        if url.is_empty() || url.starts_with('/') { continue; }
        // Extract title (text between > and </a>)
        let title = extract_tag_text(chunk);
        // Look for snippet in the rest
        let snippet = if let Some(snip_start) = chunk.find("result__snippet") {
            let snip = &chunk[snip_start..];
            strip_html(&extract_tag_content(snip))
        } else {
            String::new()
        };
        if !title.is_empty() {
            results.push((strip_html(&title), snippet, url));
        }
    }
    results
}

fn extract_attr(s: &str, attr: &str) -> String {
    let Some(idx) = s.find(attr) else { return String::new() };
    let rest = &s[idx + attr.len()..];
    let end = rest.find('"').unwrap_or(rest.len());
    rest[..end].to_string()
}

fn extract_tag_text(s: &str) -> String {
    let Some(gt) = s.find('>') else { return String::new() };
    let rest = &s[gt + 1..];
    let end = rest.find("</").unwrap_or(rest.len());
    rest[..end].to_string()
}

fn extract_tag_content(s: &str) -> String {
    let Some(gt) = s.find('>') else { return String::new() };
    let rest = &s[gt + 1..];
    let end = rest.find("</").unwrap_or(rest.len());
    rest[..end].to_string()
}
