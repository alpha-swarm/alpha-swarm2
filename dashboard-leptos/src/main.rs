//! alpha-swarm Leptos CSR dashboard. Talks to the daemon's /sql shim
//! (same-origin :8001). Polls every POLL_MS; degrades to empty on any error
//! (never panics the UI).

use gloo_net::http::Request;
use gloo_timers::callback::Interval;
use leptos::*;
use serde_json::Value;
use std::collections::HashSet;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const SQL_URL: &str = "/sql";
const POLL_MS: u32 = 3000;

// --- Graph canvas geometry + force-layout tuning (Obsidian-style view) ---
const GW: f64 = 1180.0;
const GH: f64 = 760.0;
const GRAPH_ITERS: usize = 280;
const DIM_OPACITY: f64 = 0.10;
const EDGE_OPACITY: f64 = 0.22;

// --- 3D graph bridge (graph-bridge.js → 3d-force-graph / three.js). The JS
// globals load in index.html before the WASM bundle, so these are safe to call
// from Leptos effects (which run after mount). The bridge buffers data until
// its WebGL instance exists and routes node clicks back via setOnClick. ---
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = AlphaGraph, js_name = init)]
    fn graph3d_init(container_id: &str);
    #[wasm_bindgen(js_namespace = AlphaGraph, js_name = setData)]
    fn graph3d_set_data(nodes_json: &str, links_json: &str);
    #[wasm_bindgen(js_namespace = AlphaGraph, js_name = setOnClick)]
    fn graph3d_set_on_click(cb: &Closure<dyn FnMut(f64)>);
}

/// POST a SurrealQL statement; return the first statement's result rows.
async fn sql(query: String) -> Vec<Value> {
    let built = Request::post(SQL_URL)
        .header("Content-Type", "text/plain")
        .body(query);
    let Ok(req) = built else { return vec![] };
    let Ok(resp) = req.send().await else { return vec![] };
    let Ok(json) = resp.json::<Value>().await else { return vec![] };
    // SurrealDB /sql shape: [{ "status":"OK", "result":[...] }, ...]
    json.as_array()
        .and_then(|arr| arr.first())
        .and_then(|stmt| stmt.get("result"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
}

/// GET a JSON endpoint (e.g. /review); Null on any error.
async fn get_json(path: &str) -> Value {
    match Request::get(path).send().await {
        Ok(resp) => resp.json::<Value>().await.unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

/// String field accessor (handles string + non-string JSON values).
fn field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn short_time(ts: &str) -> String {
    ts.chars().take(19).collect::<String>().replace('T', " ")
}

/// Human "x ago" for an ISO timestamp, via the browser clock — shows whether a
/// run is actually live (recent) or stalled (minutes old).
fn age(ts: &str) -> String {
    if ts.is_empty() {
        return "—".into();
    }
    let t = js_sys::Date::new(&JsValue::from_str(ts)).get_time();
    if t.is_nan() {
        return "—".into();
    }
    let secs = ((js_sys::Date::now() - t) / 1000.0).max(0.0) as u64;
    if secs < 90 {
        format!("{secs}s ago")
    } else if secs < 5400 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Graph view — goals ↔ agents ↔ SONA ↔ knowledge, force-directed (Obsidian-ish)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct GNode {
    id: String,
    kind: String,
    label: String,
    status: String,
    /// secondary line: model (run) / "Nx reused" (pattern) / "N symbols" (file).
    sub: String,
    /// stable identity for detail lookups: run id / pattern key / file path.
    ident: String,
}

#[derive(Clone)]
struct GraphData {
    nodes: Vec<GNode>,
    edges: Vec<(usize, usize, bool)>, // (src, dst, ok)
    pos: Vec<(f64, f64)>,
    adj: Rc<Vec<HashSet<usize>>>,
    deg: Vec<usize>,
}

/// Fill colour per node kind (matches the legend).
fn kind_color(kind: &str, status: &str) -> &'static str {
    match kind {
        "goal" => "#e0af68",
        "run" => match status {
            "passed" => "#9ece6a",
            "failed" => "#f7768e",
            "running" => "#7aa2f7",
            "skipped" | "cancelled" => "#565f89",
            _ => "#a9b1d6",
        },
        "pattern" => "#bb9af7",
        "trajectory" => "#7dcfff",
        "file" => "#7c8099",
        _ => "#a9b1d6",
    }
}

/// Legend bucket a node belongs to — the toggle key for show/hide. Runs split
/// by status (matching the legend rows); everything else is keyed by kind.
fn node_key(kind: &str, status: &str) -> &'static str {
    match kind {
        "goal" => "goal",
        "run" => match status {
            "passed" => "run-pass",
            "failed" => "run-fail",
            "running" => "run-running",
            _ => "run-other", // skipped/cancelled: no legend row → not toggleable
        },
        "pattern" => "pattern",
        "trajectory" => "trajectory",
        "file" => "file",
        _ => "other",
    }
}

/// Serialize the graph for the 3D bridge: nodes keyed by index (id), links by
/// (source,target) index. The library runs its own 3D force layout, so the 2D
/// `pos` is unused here. Colour + degree mirror the 2D legend/sizing. Nodes
/// whose legend bucket is in `hidden` (and edges touching them) are dropped;
/// surviving nodes keep their original index as id, so clicks still map back.
fn graph_json(g: &GraphData, hidden: &HashSet<String>) -> (String, String) {
    let vis = |i: usize| !hidden.contains(node_key(&g.nodes[i].kind, &g.nodes[i].status));
    let nodes: Vec<Value> = g.nodes.iter().enumerate().filter(|(i, _)| vis(*i)).map(|(i, n)| serde_json::json!({
        "id": i,
        "label": n.label,
        "kind": n.kind,
        "sub": n.sub,
        "color": kind_color(&n.kind, &n.status),
        "deg": g.deg.get(i).copied().unwrap_or(0),
    })).collect();
    let links: Vec<Value> = g.edges.iter().filter(|(s, d, _)| vis(*s) && vis(*d)).map(|&(s, d, ok)| serde_json::json!({
        "source": s, "target": d, "ok": ok,
    })).collect();
    (Value::Array(nodes).to_string(), Value::Array(links).to_string())
}

/// Fruchterman-Reingold force-directed layout. Deterministic (nodes seeded on a
/// circle by index — no RNG), cooled over `GRAPH_ITERS`. O(n²) per iter; node
/// counts are LIMITed server-side so this stays well under a frame budget and
/// runs once per load (not per poll), so the graph doesn't jump around.
fn layout(n: usize, edges: &[(usize, usize, bool)]) -> Vec<(f64, f64)> {
    let mut pos: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = i as f64 / n.max(1) as f64 * std::f64::consts::TAU;
            (GW / 2.0 + GW * 0.34 * a.cos(), GH / 2.0 + GH * 0.34 * a.sin())
        })
        .collect();
    if n < 2 {
        return pos;
    }
    let k = 0.75 * ((GW * GH) / n as f64).sqrt();
    let mut temp = GW * 0.10;
    for _ in 0..GRAPH_ITERS {
        let mut disp = vec![(0.0_f64, 0.0_f64); n];
        // Repulsion between every pair.
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = k * k / d;
                let (ux, uy) = (dx / d, dy / d);
                disp[i].0 += ux * f;
                disp[i].1 += uy * f;
                disp[j].0 -= ux * f;
                disp[j].1 -= uy * f;
            }
        }
        // Attraction along edges.
        for &(a, b, _) in edges {
            let dx = pos[a].0 - pos[b].0;
            let dy = pos[a].1 - pos[b].1;
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            let f = d * d / k;
            let (ux, uy) = (dx / d, dy / d);
            disp[a].0 -= ux * f;
            disp[a].1 -= uy * f;
            disp[b].0 += ux * f;
            disp[b].1 += uy * f;
        }
        // Apply, capped by temperature, clamped to canvas.
        for i in 0..n {
            let (dx, dy) = disp[i];
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            let m = d.min(temp);
            pos[i].0 = (pos[i].0 + dx / d * m).clamp(28.0, GW - 28.0);
            pos[i].1 = (pos[i].1 + dy / d * m).clamp(28.0, GH - 28.0);
        }
        temp *= 0.965;
    }
    pos
}

/// Fetch /graph, parse, lay out, build adjacency. Runs once per refresh.
async fn load_graph() -> GraphData {
    let v = get_json("/graph").await;
    let nraw = v.get("nodes").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let eraw = v.get("edges").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let nodes: Vec<GNode> = nraw.iter().map(|n| {
        let kind = field(n, "kind");
        let sub = match kind.as_str() {
            "run" => field(n, "model"),
            "pattern" => format!("{}× reused", n.get("uses").and_then(|v| v.as_i64()).unwrap_or(0)),
            "file" => format!("{} symbols", n.get("symbols").and_then(|v| v.as_i64()).unwrap_or(0)),
            _ => String::new(),
        };
        let id = field(n, "id");
        let ident = match kind.as_str() {
            "pattern" => id.strip_prefix("pattern:").unwrap_or(&id).to_string(),
            "file" => field(n, "path"),
            _ => id.clone(),
        };
        GNode { id, kind, label: field(n, "label"), status: field(n, "status"), sub, ident }
    }).collect();

    let index: std::collections::HashMap<String, usize> =
        nodes.iter().enumerate().map(|(i, n)| (n.id.clone(), i)).collect();
    let mut edges: Vec<(usize, usize, bool)> = Vec::new();
    for e in &eraw {
        let (s, d) = (field(e, "src"), field(e, "dst"));
        if let (Some(&si), Some(&di)) = (index.get(&s), index.get(&d)) {
            let ok = e.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
            edges.push((si, di, ok));
        }
    }
    let n = nodes.len();
    let pos = layout(n, &edges);
    let mut adj = vec![HashSet::new(); n];
    for &(a, b, _) in &edges {
        adj[a].insert(b);
        adj[b].insert(a);
    }
    let deg = adj.iter().map(|a| a.len()).collect();
    GraphData { nodes, edges, pos, adj: Rc::new(adj), deg }
}

/// Fetch the detail payload for a clicked node. Runs get their full row + plan
/// (so detail_view can show diff + agent output); patterns get their content;
/// files get their symbol list. Tagged with "mode" for the renderer.
async fn fetch_node_detail(kind: String, ident: String) -> Value {
    let id = ident.replace('\'', "");
    match kind.as_str() {
        "run" => {
            let run = sql(format!("SELECT * FROM {id}")).await.into_iter().next().unwrap_or(Value::Null);
            let plan = sql(format!(
                "SELECT sub_tasks, version FROM goal_plan WHERE run_id = '{id}' ORDER BY version DESC LIMIT 1"
            )).await.into_iter().next();
            let tasks = plan.and_then(|p| p.get("sub_tasks").and_then(|v| v.as_array()).cloned()).unwrap_or_default();
            serde_json::json!({ "mode": "run", "run": run, "tasks": tasks })
        }
        "pattern" => {
            let rows = sql(format!(
                "SELECT content, use_count FROM memory_entry WHERE namespace = 'patterns' AND key = '{id}' LIMIT 1"
            )).await;
            serde_json::json!({ "mode": "pattern", "rows": rows })
        }
        "file" => {
            let rows = sql(format!(
                "SELECT name, kind, line FROM code_entity WHERE file = '{id}' ORDER BY line LIMIT 60"
            )).await;
            serde_json::json!({ "mode": "file", "rows": rows })
        }
        _ => serde_json::json!({ "mode": "none" }),
    }
}

/// Build the force-directed SVG. Hover dims everything but the node + its
/// neighbours (Obsidian-style) and reveals their labels; click selects a node.
fn render_graph(
    g: GraphData,
    hovered: ReadSignal<Option<usize>>,
    set_hovered: WriteSignal<Option<usize>>,
    selected: ReadSignal<Option<usize>>,
    set_selected: WriteSignal<Option<usize>>,
    hidden: HashSet<String>,
) -> View {
    if g.nodes.is_empty() {
        return view! { <p class="muted">"No graph yet — submit a goal and let a run complete."</p> }.into_view();
    }
    let pos = Rc::new(g.pos.clone());
    // Per-node visibility from the legend toggles; hidden nodes + any edge
    // touching them are dropped (indices preserved for the surviving ones).
    let visible: Rc<Vec<bool>> = Rc::new(
        g.nodes.iter().map(|n| !hidden.contains(node_key(&n.kind, &n.status))).collect()
    );

    let pe = pos.clone();
    let ve = visible.clone();
    let edge_views = g.edges.iter().filter(|e| ve[e.0] && ve[e.1]).map(|&(s, d, ok)| {
        let (x1, y1) = pe[s];
        let (x2, y2) = pe[d];
        let color = if ok { "#414868" } else { "#f7768e" };
        let style = move || {
            let o = match hovered.get() {
                None => EDGE_OPACITY,
                Some(h) => if h == s || h == d { 0.85 } else { 0.03 },
            };
            format!("stroke:{color};stroke-width:1;stroke-opacity:{o}")
        };
        view! { <line x1=x1 y1=y1 x2=x2 y2=y2 style=style /> }
    }).collect_view();

    let pn = pos.clone();
    let vn = visible.clone();
    let node_views = g.nodes.iter().enumerate().filter(|(i, _)| vn[*i]).map(|(i, node)| {
        let (x, y) = pn[i];
        let r = (5.0 + (g.deg[i] as f64).sqrt() * 2.0).min(16.0);
        let ly = y - r - 4.0;
        let color = kind_color(&node.kind, &node.status);
        let label = node.label.clone();
        let label_kind = node.kind.clone();

        let adj_o = g.adj.clone();
        let circle_style = move || {
            let op = match hovered.get() {
                None => 0.95,
                Some(h) => if h == i || adj_o[h].contains(&i) { 1.0 } else { DIM_OPACITY },
            };
            let ring = if selected.get() == Some(i) { "#c0caf5" } else { "rgba(0,0,0,0)" };
            format!("fill:{color};fill-opacity:{op};stroke:{ring};stroke-width:2;cursor:pointer")
        };
        let adj_l = g.adj.clone();
        let show_label = move || match hovered.get() {
            None => label_kind == "goal",
            Some(h) => h == i || adj_l[h].contains(&i),
        };
        view! {
            <g>
                <circle cx=x cy=y r=r style=circle_style
                    on:mouseenter=move |_| set_hovered.set(Some(i))
                    on:mouseleave=move |_| set_hovered.set(None)
                    on:click=move |_| set_selected.update(|s| *s = if *s == Some(i) { None } else { Some(i) }) />
                {move || show_label().then({
                    let label = label.clone();
                    move || view! {
                        <text x=x y=ly style="fill:#c0caf5;font-size:9px;text-anchor:middle;pointer-events:none">{label}</text>
                    }
                })}
            </g>
        }
    }).collect_view();

    view! {
        <svg width=GW height=GH viewBox=format!("0 0 {GW} {GH}")
            style="max-width:100%;height:auto;background:#16161e;border-radius:8px;border:1px solid #2a2e3f">
            {edge_views}
            {node_views}
        </svg>
    }.into_view()
}

#[component]
fn GraphView() -> impl IntoView {
    let (refresh, set_refresh) = create_signal(0u32);
    let data = create_local_resource(move || refresh.get(), |_| async move { load_graph().await });
    let (hovered, set_hovered) = create_signal::<Option<usize>>(None);
    let (selected, set_selected) = create_signal::<Option<usize>>(None);
    // 3D (WebGL force graph) is the default view; toggle drops back to the 2D SVG.
    let (view_3d, set_view_3d) = create_signal(true);
    // Legend buckets toggled off (hidden). Click a legend chip to show/hide that
    // kind in both the 3D and 2D views.
    let (hidden, set_hidden) = create_signal::<HashSet<String>>(HashSet::new());

    // Register the node-click callback once: the 3D graph reports the clicked
    // node's id (= its index), which drives the SAME `selected` signal + detail
    // panel as the 2D view. Closure is leaked (lives for the app's lifetime).
    create_effect(move |prev: Option<()>| {
        if prev.is_none() {
            let cb = Closure::wrap(Box::new(move |id: f64| {
                set_selected.set(Some(id as usize));
            }) as Box<dyn FnMut(f64)>);
            graph3d_set_on_click(&cb);
            cb.forget();
        }
    });

    // Push graph data into the 3D instance whenever the data or view changes.
    // Re-runs on refresh (data) and on toggling back to 3D; init is idempotent
    // and the bridge buffers data until its container <div> is mounted.
    create_effect(move |_| {
        if !view_3d.get() { return; }
        let Some(g) = data.get() else { return; };
        let h = hidden.get();                       // re-runs when legend toggled
        let (nodes_json, links_json) = graph_json(&g, &h);
        graph3d_init("graph3d");
        graph3d_set_data(&nodes_json, &links_json);
    });

    // Detail of the selected node (keyed on selection + the loaded graph).
    let detail = create_local_resource(
        move || data.get().and_then(|g| {
            let i = selected.get()?;
            g.nodes.get(i).map(|n| (n.kind.clone(), n.ident.clone()))
        }),
        |key| async move {
            match key {
                Some((kind, ident)) => fetch_node_detail(kind, ident).await,
                None => Value::Null,
            }
        },
    );

    // (legend bucket key, label, colour). Key matches node_key() for toggling.
    let legend = [
        ("goal", "Goal", "#e0af68"), ("run-pass", "Agent ✓", "#9ece6a"), ("run-fail", "Agent ✗", "#f7768e"),
        ("run-running", "Running", "#7aa2f7"), ("pattern", "Pattern", "#bb9af7"),
        ("trajectory", "Trajectory", "#7dcfff"), ("file", "File", "#7c8099"),
    ];

    view! {
        <div class="graph-section">
        <div class="graph-main panel">
            <div class="row" style="justify-content:space-between; align-items:center">
                <h2 style="margin:0">"Graph — goals ↔ agents ↔ SONA ↔ knowledge"</h2>
                <div class="row" style="gap:14px; align-items:center">
                    <span class="muted">{move || {
                        let n = data.get().map(|g| g.nodes.len()).unwrap_or(0);
                        let e = data.get().map(|g| g.edges.len()).unwrap_or(0);
                        format!("{n} nodes · {e} edges")
                    }}</span>
                    <button on:click=move |_| set_view_3d.update(|v| *v = !*v)>
                        {move || if view_3d.get() { "▦ 2D" } else { "✸ 3D" }}
                    </button>
                    <button on:click=move |_| set_refresh.update(|r| *r += 1)>"⟳ refresh"</button>
                </div>
            </div>
            <div class="row" style="gap:14px; flex-wrap:wrap; margin:8px 0 12px">
                {legend.iter().map(|(key, name, col)| {
                    let key = key.to_string();
                    let (kc, kd) = (key.clone(), key.clone());
                    let off = move || hidden.get().contains(&kc);
                    let toggle = move |_| set_hidden.update(|h| { if !h.remove(&kd) { h.insert(kd.clone()); } });
                    let chip_style = move || format!(
                        "display:inline-flex;align-items:center;gap:5px;font-size:12px;cursor:pointer;user-select:none;{}",
                        if off() { "opacity:0.4;text-decoration:line-through" } else { "" }
                    );
                    let dot_style = format!("width:10px;height:10px;border-radius:50%;background:{col};display:inline-block");
                    view! {
                        <span class="muted" style=chip_style on:click=toggle title="click to show/hide">
                            <span style=dot_style></span>
                            {*name}
                        </span>
                    }
                }).collect_view()}
            </div>
            {move || {
                if view_3d.get() {
                    // 3D: the bridge attaches a WebGL canvas to this div and runs
                    // its own force layout. Drag to orbit, scroll to zoom, click a
                    // node to fly in + inspect it in the panel below.
                    view! {
                        <div id="graph3d"
                            style="width:100%;background:#0d1117;border-radius:8px;border:1px solid #2a2e3f;overflow:hidden"></div>
                    }.into_view()
                } else {
                    match data.get() {
                        None => view! { <p class="muted">"laying out graph…"</p> }.into_view(),
                        Some(g) => render_graph(g, hovered, set_hovered, selected, set_selected, hidden.get()),
                    }
                }
            }}
        </div>
        <aside class="inspector panel">
            <h2>"Inspector"</h2>
                {move || {
                    let Some(idx) = selected.get() else {
                        return view! { <p class="muted">"Click a node to inspect it — runs show plan + diff + raw output; patterns show content; files show symbols."</p> }.into_view();
                    };
                    let Some(g) = data.get() else { return ().into_view(); };
                    let Some(node) = g.nodes.get(idx).cloned() else { return ().into_view(); };
                    let neighbors: Vec<(usize, GNode)> = g.adj[idx].iter()
                        .filter_map(|&j| g.nodes.get(j).map(|n| (j, n.clone()))).collect();
                    let det = detail.get().unwrap_or(Value::Null);
                    let body = match field(&det, "mode").as_str() {
                        "run" => {
                            let run = det.get("run").cloned().unwrap_or(Value::Null);
                            let tasks = det.get("tasks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            if run.is_null() {
                                view! { <span class="muted">"loading…"</span> }.into_view()
                            } else {
                                detail_view(run, tasks)
                            }
                        }
                        "pattern" => {
                            let rows = det.get("rows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            let content = rows.first().map(|r| field(r, "content")).unwrap_or_default();
                            view! { <div><b>"Distilled pattern"</b>
                                <pre style="white-space:pre-wrap; max-height:280px; overflow:auto">{content}</pre></div> }.into_view()
                        }
                        "file" => {
                            let rows = det.get("rows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                            view! { <div><b>"Symbols (code_entity)"</b>
                                <ul style="max-height:240px; overflow:auto">
                                    {rows.iter().map(|r| view! {
                                        <li class="muted">{field(r, "kind")}" "<b style="color:var(--fg)">{field(r, "name")}</b>" :"{field(r, "line")}</li>
                                    }).collect_view()}
                                </ul></div> }.into_view()
                        }
                        _ => view! { <span class="muted">"loading…"</span> }.into_view(),
                    };
                    view! {
                        <div>
                            <div class="row" style="gap:10px; align-items:center; margin-bottom:6px">
                                <span style=format!("width:12px;height:12px;border-radius:50%;background:{};display:inline-block", kind_color(&node.kind, &node.status))></span>
                                <b>{node.kind.clone()}</b>
                                <span class="goal">{node.label.clone()}</span>
                                {(!node.sub.is_empty()).then(|| view! { <span class="muted">"· "{node.sub.clone()}</span> })}
                            </div>
                            <div class="muted" style="margin-bottom:8px; font-size:12px">
                                <b>"Connected ("{neighbors.len()}"): "</b>
                                {neighbors.into_iter().map(|(j, n)| {
                                    let dot = kind_color(&n.kind, &n.status);
                                    view! {
                                        <span style="cursor:pointer; margin-right:8px"
                                            on:click=move |_| set_selected.set(Some(j))>
                                            <span style=format!("width:8px;height:8px;border-radius:50%;background:{dot};display:inline-block;margin-right:3px")></span>
                                            {truncate(&n.label, 28)}
                                        </span>
                                    }
                                }).collect_view()}
                            </div>
                            {body}
                        </div>
                    }.into_view()
                }}
        </aside>
        </div>
    }
}

/// Which section the sidebar nav is showing in the content pane.
#[derive(Clone, Copy, PartialEq)]
enum Section { Graph, Runs, Queue, Knowledge, Review }

#[component]
fn App() -> impl IntoView {
    let (tick, set_tick) = create_signal(0u32);
    let (section, set_section) = create_signal(Section::Graph);
    // Fallback poll.
    Interval::new(POLL_MS, move || set_tick.update(|t| *t += 1)).forget();
    // Live: bump tick on each swarm event (real-time refresh; poll is the
    // fallback if SSE drops). Best-effort — ignore if EventSource is unavailable.
    if let Ok(es) = web_sys::EventSource::new("/events") {
        let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |_e: web_sys::MessageEvent| {
            set_tick.update(|t| *t += 1);
        });
        es.set_onmessage(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
        std::mem::forget(es); // keep the connection open for the app lifetime
    }
    let nav = move |s: Section, label: &'static str| view! {
        <button class="nav-item" class:active=move || section.get() == s
            on:click=move |_| set_section.set(s)>{label}</button>
    };
    view! {
        <header>
            <span class="dot"></span>
            <h1>"alpha-swarm"</h1>
            <span class="muted">"local agent swarm — live"</span>
        </header>
        <div class="app">
            <aside class="sidebar">
                {nav(Section::Graph, "◉ Graph")}
                {nav(Section::Runs, "▤ Runs")}
                {nav(Section::Queue, "▸ Queue")}
                {nav(Section::Knowledge, "✦ Knowledge")}
                {nav(Section::Review, "⎇ Review")}
                <div class="sidebar-foot">
                    <SubmitGoal set_tick=set_tick/>
                </div>
            </aside>
            <section class="content">
                {move || match section.get() {
                    Section::Graph => view! { <GraphView/> }.into_view(),
                    Section::Runs => view! { <Runs tick=tick/> }.into_view(),
                    Section::Queue => view! { <Queue tick=tick/> }.into_view(),
                    Section::Knowledge => view! {
                        <div class="grid2"><Routing tick=tick/><Recent tick=tick/></div>
                        <Provenance tick=tick/>
                        <Memory tick=tick/>
                    }.into_view(),
                    Section::Review => view! { <Review tick=tick/> }.into_view(),
                }}
            </section>
        </div>
    }
}

#[component]
fn Provenance(tick: ReadSignal<u32>) -> impl IntoView {
    // Learning provenance: each distilled pattern + the runs that derived from
    // it (pattern_effectiveness links a run to the pattern that guided it).
    let patterns = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT key, content, use_count FROM memory_entry WHERE namespace = 'patterns' ORDER BY use_count DESC LIMIT 20".into()).await
        },
    );
    let links = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT pattern_id, run_id, run_succeeded FROM pattern_effectiveness LIMIT 500".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Provenance — runs derived from each learned pattern"</h2>
            {move || {
                let lks = links.get().unwrap_or_default();
                let pats = patterns.get().unwrap_or_default();
                if pats.is_empty() {
                    return view! { <p class="muted">"No distilled patterns yet — they appear once successful runs are learned from."</p> }.into_view();
                }
                pats.into_iter().map(|p| {
                    let key = field(&p, "key");
                    let runs: Vec<&Value> = lks.iter()
                        .filter(|l| field(l, "pattern_id").ends_with(&key))
                        .collect();
                    let uc = p.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0);
                    view! {
                        <details style="margin-bottom:6px">
                            <summary style="cursor:pointer">
                                {truncate(&field(&p, "content"), 110)}
                                <span class="muted">" — guided "{runs.len()}" run(s), reused "{uc}"×"</span>
                            </summary>
                            <ul>
                                {runs.iter().map(|l| {
                                    let ok = l.get("run_succeeded").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let mark = if ok { "✓" } else { "✗" };
                                    let color = if ok { "var(--ok)" } else { "var(--fail)" };
                                    view! {
                                        <li class="muted">
                                            <span style=format!("color:{color}")>{mark}</span>
                                            " "{field(l, "run_id")}
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        </details>
                    }
                }).collect_view().into_view()
            }}
        </div>
    }
}

#[component]
fn Review(tick: ReadSignal<u32>) -> impl IntoView {
    let data = create_local_resource(move || tick.get(), |_| async move { get_json("/review").await });
    let commits = move || data.get().as_ref()
        .and_then(|d| d.get("commits")).and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let prs = move || data.get().as_ref()
        .and_then(|d| d.get("prs")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    view! {
        <div class="panel">
            <h2>"Review — swarm/auto commits + open PRs"</h2>
            <div class="grid2">
                <div>
                    <b>"Loop commits (not yet in main)"</b>
                    <ul>
                        {move || commits().into_iter().filter_map(|c| c.as_str().map(String::from)).map(|c| view! {
                            <li class="muted" style="font-family:ui-monospace,monospace; font-size:12px">{c}</li>
                        }).collect_view()}
                    </ul>
                </div>
                <div>
                    <b>"Open PRs"</b>
                    <ul>
                        {move || prs().into_iter().map(|p| {
                            let n = p.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                            view! {
                                <li>"#"{n}" "{field(&p, "title")}
                                    <span class="muted">" ("{field(&p, "headRefName")}")"</span></li>
                            }
                        }).collect_view()}
                    </ul>
                </div>
            </div>
        </div>
    }
}

/// Render a run's expanded detail (model/tokens/duration, plan, files, gate, diff).
fn detail_view(r: Value, tasks: Vec<Value>) -> View {
    let files = r.get("files_modified").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let err = field(&r, "error_message");
    let diff = field(&r, "diff");
    let output = field(&r, "response_text");
    let num = |k: &str| r.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    view! {
        <div style="padding:6px 0 10px">
            <div class="row" style="gap:18px; margin-bottom:10px">
                <span class="muted">"model: "{field(&r, "model_used")}</span>
                <span class="muted">"tokens: "{num("tokens_input")}" / "{num("tokens_output")}</span>
                <span class="muted">{format!("{}ms", num("duration_ms"))}</span>
            </div>
            <b>"Plan"</b>
            <ul>
                {tasks.iter().map(|t| view! {
                    <li>{field(t, "id")}": "{field(t, "description")}
                        <span class="muted">" ["{field(t, "complexity")}"]"</span></li>
                }).collect_view()}
            </ul>
            <b>"Files modified"</b>
            <ul>
                {files.iter().filter_map(|f| f.as_str().map(String::from)).map(|f| view! {
                    <li class="muted">{f}</li>
                }).collect_view()}
            </ul>
            {(!err.is_empty()).then(|| view! {
                <div><b style="color:var(--fail)">"Gate / failure"</b>
                    <pre style="white-space:pre-wrap; color:var(--fail)">{err}</pre></div>
            })}
            {(!diff.is_empty()).then(|| view! {
                <div><b>"Diff"</b>
                    <pre style="white-space:pre-wrap; max-height:300px; overflow:auto">{truncate(&diff, 4000)}</pre></div>
            })}
            {(!output.is_empty()).then(|| view! {
                <div><b>"Agent output"</b>
                    <pre style="white-space:pre-wrap; max-height:260px; overflow:auto; color:var(--muted)">{truncate(&output, 4000)}</pre></div>
            })}
        </div>
    }.into_view()
}

#[component]
fn Memory(tick: ReadSignal<u32>) -> impl IntoView {
    let counts = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT namespace, count() AS c FROM memory_entry GROUP BY namespace".into()).await
        },
    );
    let patterns = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT content, use_count FROM memory_entry WHERE namespace = 'patterns' ORDER BY use_count DESC LIMIT 12".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Learned memory (SONA)"</h2>
            <div class="row" style="gap:16px; margin-bottom:12px">
                {move || counts.get().unwrap_or_default().into_iter().map(|r| {
                    let c = r.get("c").and_then(|v| v.as_i64()).unwrap_or(0);
                    view! { <span class="muted">{field(&r, "namespace")}": "<b style="color:var(--fg)">{c}</b></span> }
                }).collect_view()}
            </div>
            <table>
                <thead><tr><th>"Distilled pattern"</th><th>"Reused"</th></tr></thead>
                <tbody>
                    {move || patterns.get().unwrap_or_default().into_iter().map(|r| {
                        let uc = r.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0);
                        view! {
                            <tr>
                                <td class="goal">{truncate(&field(&r, "content"), 160)}</td>
                                <td class="muted">{format!("{uc}×")}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn SubmitGoal(set_tick: WriteSignal<u32>) -> impl IntoView {
    let (project, set_project) = create_signal("alpha-swarm2".to_string());
    let (goal, set_goal) = create_signal(String::new());
    let (status, set_status) = create_signal(String::new());

    let submit = move |_| {
        let p = project.get().replace('\'', "");
        let g = goal.get();
        if g.trim().is_empty() {
            set_status.set("goal is empty".into());
            return;
        }
        let ge = g.replace('\'', "");
        set_status.set("submitting…".into());
        spawn_local(async move {
            let q = format!(
                "CREATE autopilot_goal SET project = '{p}', goal = '{ge}', status = 'queued', created_at = time::now()"
            );
            let _ = sql(q).await;
            set_status.set("queued ✓".into());
            set_goal.set(String::new());
            set_tick.update(|t| *t += 1);
        });
    };

    view! {
        <div class="panel">
            <h2>"Submit goal"</h2>
            <div class="row" style="margin-bottom:10px">
                <input
                    prop:value=move || project.get()
                    on:input=move |e| set_project.set(event_target_value(&e))
                    style="width:220px"/>
                <span class="muted">{move || status.get()}</span>
            </div>
            <textarea
                prop:value=move || goal.get()
                on:input=move |e| set_goal.set(event_target_value(&e))
                placeholder="e.g. Add a doc comment to the X function in crates/.../y.rs"></textarea>
            <div class="row" style="margin-top:10px">
                <button on:click=submit>"Queue goal"</button>
            </div>
        </div>
    }
}

#[component]
fn Queue(tick: ReadSignal<u32>) -> impl IntoView {
    let q = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT goal, project, created_at FROM autopilot_goal WHERE status = 'queued' ORDER BY created_at ASC LIMIT 25".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Queue — submitted, awaiting a run slot"</h2>
            {move || {
                let rows = q.get().unwrap_or_default();
                if rows.is_empty() {
                    view! { <p class="muted">"Empty. Submitted goals land here, then move to Runs when the loop picks them up."</p> }.into_view()
                } else {
                    view! {
                        <ul>
                            {rows.into_iter().map(|r| view! {
                                <li>{truncate(&field(&r, "goal"), 110)}
                                    <span class="muted">" — "{short_time(&field(&r, "created_at"))}</span></li>
                            }).collect_view()}
                        </ul>
                    }.into_view()
                }
            }}
        </div>
    }
}

#[component]
fn Runs(tick: ReadSignal<u32>) -> impl IntoView {
    let (expanded, set_expanded) = create_signal::<Option<String>>(None);
    let runs = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT id, status, task_description, progress_message, created_at, last_activity_at FROM agent_run ORDER BY created_at DESC LIMIT 25".into()).await
        },
    );
    // Detail for the currently-expanded run; refetches on expand change + tick.
    let detail = create_local_resource(
        move || (expanded.get(), tick.get()),
        |(sel, _)| async move {
            let Some(id) = sel else { return None };
            let run = sql(format!("SELECT * FROM {id}")).await.into_iter().next();
            // SurrealDB requires ORDER BY fields in the projection.
            let plan = sql(format!(
                "SELECT sub_tasks, version FROM goal_plan WHERE run_id = '{id}' ORDER BY version DESC LIMIT 1"
            )).await.into_iter().next();
            let tasks = plan
                .and_then(|p| p.get("sub_tasks").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default();
            run.map(|r| (r, tasks))
        },
    );
    view! {
        <div class="panel">
            <h2>"Runs — click a row to expand"</h2>
            <table>
                <thead><tr><th style="width:16px"></th><th>"Status"</th><th>"Goal"</th><th>"Progress"</th><th>"Activity"</th><th>"When"</th></tr></thead>
                <tbody>
                    {move || runs.get().unwrap_or_default().into_iter().map(|r| {
                        let st = field(&r, "status");
                        let cls = format!("badge s-{st}");
                        let id = field(&r, "id");
                        let (id_click, id_caret, id_cond) = (id.clone(), id.clone(), id.clone());
                        view! {
                            <tr style="cursor:pointer" on:click=move |_| {
                                set_expanded.update(|e| {
                                    *e = if e.as_deref() == Some(id_click.as_str()) { None } else { Some(id_click.clone()) };
                                });
                            }>
                                <td class="muted">{move || if expanded.get().as_deref() == Some(id_caret.as_str()) { "▾" } else { "▸" }}</td>
                                <td><span class=cls>{st}</span></td>
                                <td class="goal">{truncate(&field(&r, "task_description"), 90)}</td>
                                <td class="muted">{truncate(&field(&r, "progress_message"), 60)}</td>
                                <td class="muted">{age(&field(&r, "last_activity_at"))}</td>
                                <td class="muted">{short_time(&field(&r, "created_at"))}</td>
                            </tr>
                            {move || (expanded.get().as_deref() == Some(id_cond.as_str())).then(|| view! {
                                <tr><td></td><td colspan="5">
                                    {move || match detail.get().flatten() {
                                        Some((rr, tasks)) => detail_view(rr, tasks),
                                        None => view! { <span class="muted">"loading…"</span> }.into_view(),
                                    }}
                                </td></tr>
                            })}
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn Routing(tick: ReadSignal<u32>) -> impl IntoView {
    let stats = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT shape, tier, attempts, successes FROM routing_stats ORDER BY shape".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Learned routing (UCB)"</h2>
            <table>
                <thead><tr><th>"Shape"</th><th>"Tier"</th><th>"Pass"</th><th>"N"</th></tr></thead>
                <tbody>
                    {move || stats.get().unwrap_or_default().into_iter().map(|r| {
                        let a = r.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
                        let su = r.get("successes").and_then(|v| v.as_i64()).unwrap_or(0);
                        let rate = if a > 0 { format!("{:.0}%", su as f64 / a as f64 * 100.0) } else { "—".into() };
                        view! {
                            <tr>
                                <td>{field(&r, "shape")}</td>
                                <td>{field(&r, "tier")}</td>
                                <td>{rate}</td>
                                <td class="muted">{a}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn Recent(tick: ReadSignal<u32>) -> impl IntoView {
    let fails = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT task_description, error_message, created_at FROM agent_run WHERE status = 'failed' ORDER BY created_at DESC LIMIT 8".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Recent rejects"</h2>
            <table>
                <thead><tr><th>"Goal"</th><th>"Reason"</th></tr></thead>
                <tbody>
                    {move || fails.get().unwrap_or_default().into_iter().map(|r| {
                        view! {
                            <tr>
                                <td class="goal">{truncate(&field(&r, "task_description"), 70)}</td>
                                <td class="muted">{truncate(&field(&r, "error_message"), 120)}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

fn main() {
    mount_to_body(App);
}
