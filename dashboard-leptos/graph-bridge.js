// 3D graph bridge: thin glue between the Leptos/WASM dashboard and the
// `3d-force-graph` library (WebGL via three.js, same lib engram uses). The
// Rust side calls window.AlphaGraph.{init,setData,setOnClick}; node clicks are
// routed back into Rust to drive the same detail panel the 2D SVG view uses.
//
// Loaded as a plain <script> in index.html BEFORE the WASM bundle, so the
// global exists by the time Rust effects run. No build step (vendored UMD).
(function () {
  var graph = null;       // ForceGraph3D instance
  var container = null;   // bound DOM element
  var onClick = null;     // Rust callback (node id -> ())
  var pending = null;     // graphData buffered until the instance exists

  // The container <div> is created by Leptos AFTER mount; retry on the next
  // animation frame until it is in the DOM (and the library is loaded).
  function ensure(id) {
    if (typeof ForceGraph3D === "undefined") {
      requestAnimationFrame(function () { ensure(id); });
      return;
    }
    var el = document.getElementById(id);
    if (!el) {
      requestAnimationFrame(function () { ensure(id); });
      return;
    }
    if (graph && container === el) return; // already initialised on this node

    container = el;
    graph = ForceGraph3D()(el)
      .backgroundColor("#0d1117")
      .showNavInfo(false)
      .nodeColor(function (n) { return n.color || "#a9b1d6"; })
      .nodeVal(function (n) { return Math.max(1, Math.sqrt((n.deg || 0) + 1) * 2); })
      .nodeOpacity(0.95)
      .nodeResolution(12)
      .nodeLabel(function (n) {
        var sub = n.sub ? " &middot; " + n.sub : "";
        return '<div style="background:#161b22;border:1px solid #30363d;border-radius:6px;' +
          'padding:6px 9px;color:#c9d1d9;font:12px sans-serif;max-width:280px">' +
          '<b>' + escapeHtml(n.label) + '</b><br>' +
          '<span style="color:#8b949e">' + escapeHtml(n.kind) + escapeHtml(sub) + '</span></div>';
      })
      .linkColor(function (l) { return l.ok === false ? "#f7768e" : "#414868"; })
      .linkOpacity(0.4)
      .linkWidth(0.6)
      .onNodeClick(function (n) {
        // Fly the camera in to the clicked node, keeping it centred.
        var dist = 90;
        var hyp = Math.hypot(n.x || 0, n.y || 0, n.z || 0) || 1;
        var r = 1 + dist / hyp;
        graph.cameraPosition(
          { x: (n.x || 0) * r, y: (n.y || 0) * r, z: (n.z || 0) * r },
          n, 1100
        );
        if (onClick) onClick(n.id);
      });

    var resize = function () {
      if (!container) return;
      // Size the canvas to the CONTAINER (3d-force-graph otherwise defaults to
      // the full window, which overflows the panel).
      graph.width(container.clientWidth);
      graph.height(container.clientHeight);
    };
    resize();
    requestAnimationFrame(resize);   // re-fit once layout/clamp has settled
    setTimeout(resize, 150);         // and once more after first paint
    window.addEventListener("resize", resize);

    if (pending) { graph.graphData(pending); pending = null; }
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  window.AlphaGraph = {
    init: function (id) { ensure(id); },
    setData: function (nodesJson, linksJson) {
      var data = { nodes: JSON.parse(nodesJson), links: JSON.parse(linksJson) };
      if (graph) graph.graphData(data); else pending = data;
    },
    setOnClick: function (fn) { onClick = fn; },
  };
})();
