///! HolyContainer Web Dashboard — embedded HTTP server + REST API + Web UI.
///! Serves a stunning dark-mode dashboard on localhost.
///! Zero dependencies — hand-rolled HTTP server using raw TCP sockets.

use std::collections::HashMap;
use std::io::{Read, Write, BufReader, BufRead};
use std::net::TcpListener;
use std::path::PathBuf;
use std::fs;

use crate::config;
use crate::error::{ContainerError, Result};

// ─── Dashboard Server ───────────────────────────────────────────────────────

/// Start the dashboard web server on the given port.
pub fn start_dashboard(port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr)
        .map_err(|e| ContainerError::Network(format!("bind {}: {}", addr, e)))?;

    println!("╔════════════════════════════════════════════════════════╗");
    println!("║         HOLYCONTAINER DASHBOARD v2.0                  ║");
    println!("╠════════════════════════════════════════════════════════╣");
    println!("║  🌐 Open in your browser:                             ║");
    println!("║     http://localhost:{}                             ║", port);
    println!("║                                                        ║");
    println!("║  Press Ctrl+C to stop the dashboard                    ║");
    println!("╚════════════════════════════════════════════════════════╝");

    // Try to open browser automatically
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", &format!("http://localhost:{}", port)])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(format!("http://localhost:{}", port))
            .spawn();
    }

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(e) = handle_request(&mut stream) {
                    eprintln!("[dashboard] request error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("[dashboard] accept error: {}", e);
            }
        }
    }

    Ok(())
}

// ─── Request Handler ────────────────────────────────────────────────────────

fn handle_request(stream: &mut std::net::TcpStream) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line)
        .map_err(|e| ContainerError::Network(format!("read request: {}", e)))?;

    // Parse request line
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return send_response(stream, 400, "text/plain", b"Bad Request");
    }

    let method = parts[0];
    let path = parts[1];

    // Read headers (just consume them)
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|e| ContainerError::Network(format!("read header: {}", e)))?;
        if line.trim().is_empty() {
            break;
        }
        if line.to_lowercase().starts_with("content-length:") {
            content_length = line.split(':').nth(1)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
        }
    }

    // Read body if present
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)
            .map_err(|e| ContainerError::Network(format!("read body: {}", e)))?;
    }

    // Route
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            send_response(stream, 200, "text/html", DASHBOARD_HTML.as_bytes())
        }
        ("GET", "/api/containers") => {
            api_list_containers(stream)
        }
        ("GET", "/api/images") => {
            api_list_images(stream)
        }
        ("GET", "/api/system") => {
            api_system_info(stream)
        }
        ("POST", p) if p.starts_with("/api/pull/") => {
            let image = &p[10..];
            api_pull_image(stream, &urldecode(image))
        }
        ("POST", p) if p.starts_with("/api/stop/") => {
            let name = &p[10..];
            api_stop_container(stream, name)
        }
        ("POST", p) if p.starts_with("/api/rm/") => {
            let name = &p[8..];
            api_rm_container(stream, name)
        }
        _ => {
            send_response(stream, 404, "application/json", b"{\"error\":\"not found\"}")
        }
    }
}

fn urldecode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h = chars.next().unwrap_or(b'0');
            let l = chars.next().unwrap_or(b'0');
            let val = hex_byte(h) * 16 + hex_byte(l);
            result.push(val as char);
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_byte(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

fn send_response(stream: &mut std::net::TcpStream, status: u16, content_type: &str, body: &[u8]) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };

    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        status, status_text, content_type, body.len()
    );

    stream.write_all(header.as_bytes())
        .map_err(|e| ContainerError::Network(format!("write header: {}", e)))?;
    stream.write_all(body)
        .map_err(|e| ContainerError::Network(format!("write body: {}", e)))?;
    stream.flush()
        .map_err(|e| ContainerError::Network(format!("flush: {}", e)))?;

    Ok(())
}

// ─── API Endpoints ──────────────────────────────────────────────────────────

fn api_list_containers(stream: &mut std::net::TcpStream) -> Result<()> {
    let base = config::state_base_dir().join("containers");
    let mut json = String::from("[");
    let mut first = true;

    if base.exists() {
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let cfg_path = entry.path().join("config");
                    if cfg_path.exists() {
                        if let Ok(cfg) = config::ContainerConfig::load(&entry.path()) {
                            if !first { json.push(','); }
                            json.push_str(&format!(
                                "{{\"name\":\"{}\",\"state\":\"{}\",\"rootfs\":\"{}\",\"memory\":{},\"cpu\":{},\"pids\":{},\"hostname\":\"{}\",\"pid\":{}}}",
                                escape_json(&cfg.name),
                                cfg.state.as_str(),
                                escape_json(&cfg.rootfs.to_string_lossy()),
                                cfg.limits.memory_bytes,
                                cfg.limits.cpu_percent,
                                cfg.limits.max_pids,
                                escape_json(&cfg.hostname),
                                cfg.pid
                            ));
                            first = false;
                        }
                    }
                }
            }
        }
    }

    json.push(']');
    send_response(stream, 200, "application/json", json.as_bytes())
}

fn api_list_images(stream: &mut std::net::TcpStream) -> Result<()> {
    let mut json = String::from("[");
    let mut first = true;

    let base = config::state_base_dir().join("images");
    if base.exists() {
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let conf_path = entry.path().join("image.conf");
                    if conf_path.exists() {
                        if let Ok(conf) = fs::read_to_string(&conf_path) {
                            let name = conf.lines()
                                .find(|l| l.starts_with("image="))
                                .map(|l| l[6..].to_string())
                                .unwrap_or_default();
                            let layers = conf.lines()
                                .find(|l| l.starts_with("layers="))
                                .and_then(|l| l[7..].parse::<u32>().ok())
                                .unwrap_or(0);

                            // Calculate rootfs size
                            let rootfs = entry.path().join("rootfs");
                            let size = dir_size(&rootfs).unwrap_or(0);

                            if !first { json.push(','); }
                            json.push_str(&format!(
                                "{{\"name\":\"{}\",\"layers\":{},\"size\":{},\"path\":\"{}\"}}",
                                escape_json(&name), layers, size,
                                escape_json(&rootfs.to_string_lossy())
                            ));
                            first = false;
                        }
                    }
                }
            }
        }
    }

    json.push(']');
    send_response(stream, 200, "application/json", json.as_bytes())
}

fn api_system_info(stream: &mut std::net::TcpStream) -> Result<()> {
    let whp_available;
    #[cfg(target_os = "windows")]
    {
        whp_available = crate::platform::windows::whp::is_whp_available();
    }
    #[cfg(not(target_os = "windows"))]
    {
        whp_available = false;
    }

    let platform = if cfg!(target_os = "windows") { "Windows" } else { "Linux" };

    let json = format!(
        "{{\"version\":\"2.0.0\",\"platform\":\"{}\",\"whp_available\":{},\"state_dir\":\"{}\"}}",
        platform,
        whp_available,
        escape_json(&config::state_base_dir().to_string_lossy()),
    );

    send_response(stream, 200, "application/json", json.as_bytes())
}

fn api_pull_image(stream: &mut std::net::TcpStream, image: &str) -> Result<()> {
    match crate::registry::pull_image(image) {
        Ok(rootfs) => {
            let json = format!(
                "{{\"success\":true,\"rootfs\":\"{}\"}}",
                escape_json(&rootfs.to_string_lossy())
            );
            send_response(stream, 200, "application/json", json.as_bytes())
        }
        Err(e) => {
            let json = format!("{{\"success\":false,\"error\":\"{}\"}}", escape_json(&e.to_string()));
            send_response(stream, 200, "application/json", json.as_bytes())
        }
    }
}

fn api_stop_container(stream: &mut std::net::TcpStream, name: &str) -> Result<()> {
    match crate::container::stop(name) {
        Ok(()) => send_response(stream, 200, "application/json", b"{\"success\":true}"),
        Err(e) => {
            let json = format!("{{\"success\":false,\"error\":\"{}\"}}", escape_json(&e.to_string()));
            send_response(stream, 200, "application/json", json.as_bytes())
        }
    }
}

fn api_rm_container(stream: &mut std::net::TcpStream, name: &str) -> Result<()> {
    match crate::container::delete(name) {
        Ok(()) => send_response(stream, 200, "application/json", b"{\"success\":true}"),
        Err(e) => {
            let json = format!("{{\"success\":false,\"error\":\"{}\"}}", escape_json(&e.to_string()));
            send_response(stream, 200, "application/json", json.as_bytes())
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('"', "\\\"")
     .replace('\n', "\\n")
     .replace('\r', "\\r")
     .replace('\t', "\\t")
}

fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                total += dir_size(&entry.path()).unwrap_or(0);
            } else {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

// ─── Dashboard HTML ─────────────────────────────────────────────────────────

const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>HolyContainer Dashboard</title>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700;800;900&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet">
<style>
*{margin:0;padding:0;box-sizing:border-box}
:root{
  --bg-primary:#0a0e17;--bg-secondary:#111827;--bg-card:#1a1f2e;
  --bg-card-hover:#222838;--bg-glass:rgba(26,31,46,0.85);
  --accent:#6366f1;--accent-glow:#818cf8;--accent-dim:#4f46e5;
  --success:#10b981;--warning:#f59e0b;--danger:#ef4444;--info:#3b82f6;
  --text-primary:#f1f5f9;--text-secondary:#94a3b8;--text-dim:#64748b;
  --border:#1e293b;--border-glow:rgba(99,102,241,0.3);
  --gradient-1:linear-gradient(135deg,#6366f1,#8b5cf6,#a78bfa);
  --gradient-2:linear-gradient(135deg,#06b6d4,#3b82f6,#6366f1);
  --gradient-3:linear-gradient(135deg,#f59e0b,#ef4444,#ec4899);
}
body{font-family:'Inter',sans-serif;background:var(--bg-primary);color:var(--text-primary);min-height:100vh;overflow-x:hidden}
body::before{content:'';position:fixed;top:0;left:0;right:0;bottom:0;background:radial-gradient(ellipse at 20% 50%,rgba(99,102,241,0.08),transparent 60%),radial-gradient(ellipse at 80% 20%,rgba(139,92,246,0.06),transparent 50%);pointer-events:none;z-index:0}

/* Scrollbar */
::-webkit-scrollbar{width:6px}
::-webkit-scrollbar-track{background:var(--bg-primary)}
::-webkit-scrollbar-thumb{background:var(--accent-dim);border-radius:3px}

/* Header */
.header{position:sticky;top:0;z-index:100;padding:16px 32px;display:flex;align-items:center;justify-content:space-between;background:rgba(10,14,23,0.9);backdrop-filter:blur(20px);border-bottom:1px solid var(--border)}
.logo{display:flex;align-items:center;gap:12px}
.logo-icon{width:36px;height:36px;background:var(--gradient-1);border-radius:10px;display:flex;align-items:center;justify-content:center;font-size:18px;font-weight:800;color:#fff;box-shadow:0 0 20px rgba(99,102,241,0.4)}
.logo-text{font-size:20px;font-weight:700;background:var(--gradient-1);-webkit-background-clip:text;-webkit-text-fill-color:transparent}
.logo-version{font-size:11px;color:var(--text-dim);font-weight:500;padding:2px 8px;border:1px solid var(--border);border-radius:20px}
.header-actions{display:flex;gap:8px}

/* Navigation */
.nav{display:flex;gap:4px;padding:0 32px;margin-top:8px;position:relative;z-index:1}
.nav-btn{padding:10px 20px;border:none;background:transparent;color:var(--text-secondary);font-family:'Inter',sans-serif;font-size:13px;font-weight:500;cursor:pointer;border-radius:8px;transition:all .2s}
.nav-btn:hover{color:var(--text-primary);background:var(--bg-card)}
.nav-btn.active{color:var(--accent-glow);background:rgba(99,102,241,0.1);border:1px solid var(--border-glow)}

/* Main */
.main{max-width:1400px;margin:0 auto;padding:24px 32px;position:relative;z-index:1}

/* Cards */
.card{background:var(--bg-card);border:1px solid var(--border);border-radius:16px;padding:24px;margin-bottom:20px;transition:all .3s ease}
.card:hover{border-color:var(--border-glow);box-shadow:0 0 30px rgba(99,102,241,0.05)}
.card-header{display:flex;align-items:center;justify-content:space-between;margin-bottom:16px}
.card-title{font-size:16px;font-weight:600;display:flex;align-items:center;gap:8px}
.card-title span{font-size:18px}

/* Stats Grid */
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:16px;margin-bottom:24px}
.stat{background:var(--bg-card);border:1px solid var(--border);border-radius:14px;padding:20px;position:relative;overflow:hidden;transition:all .3s}
.stat:hover{transform:translateY(-2px);border-color:var(--border-glow)}
.stat::after{content:'';position:absolute;top:0;left:0;right:0;height:3px;border-radius:14px 14px 0 0}
.stat:nth-child(1)::after{background:var(--gradient-1)}
.stat:nth-child(2)::after{background:var(--gradient-2)}
.stat:nth-child(3)::after{background:linear-gradient(135deg,var(--success),#34d399)}
.stat:nth-child(4)::after{background:var(--gradient-3)}
.stat-label{font-size:12px;color:var(--text-dim);font-weight:500;text-transform:uppercase;letter-spacing:0.5px}
.stat-value{font-size:28px;font-weight:700;margin:8px 0 4px;font-family:'JetBrains Mono',monospace}
.stat-detail{font-size:12px;color:var(--text-secondary)}

/* Tables */
.table-wrap{overflow-x:auto}
table{width:100%;border-collapse:collapse}
th{text-align:left;padding:12px 16px;font-size:11px;color:var(--text-dim);font-weight:600;text-transform:uppercase;letter-spacing:0.5px;border-bottom:1px solid var(--border)}
td{padding:12px 16px;font-size:13px;border-bottom:1px solid rgba(30,41,59,0.5);color:var(--text-secondary)}
tr:hover td{background:rgba(99,102,241,0.03);color:var(--text-primary)}
.badge{display:inline-block;padding:3px 10px;border-radius:20px;font-size:11px;font-weight:600;text-transform:uppercase;letter-spacing:0.3px}
.badge-created{background:rgba(59,130,246,0.15);color:var(--info)}
.badge-running{background:rgba(16,185,129,0.15);color:var(--success)}
.badge-stopped{background:rgba(239,68,68,0.15);color:var(--danger)}

/* Buttons */
.btn{padding:8px 16px;border:none;border-radius:8px;font-family:'Inter',sans-serif;font-size:12px;font-weight:600;cursor:pointer;transition:all .2s;display:inline-flex;align-items:center;gap:6px}
.btn-primary{background:var(--accent);color:#fff}
.btn-primary:hover{background:var(--accent-glow);box-shadow:0 0 20px rgba(99,102,241,0.3)}
.btn-danger{background:rgba(239,68,68,0.15);color:var(--danger);border:1px solid rgba(239,68,68,0.2)}
.btn-danger:hover{background:rgba(239,68,68,0.25)}
.btn-success{background:rgba(16,185,129,0.15);color:var(--success);border:1px solid rgba(16,185,129,0.2)}
.btn-success:hover{background:rgba(16,185,129,0.25)}
.btn-sm{padding:5px 10px;font-size:11px}

/* Pull Input */
.pull-bar{display:flex;gap:8px;margin-bottom:20px}
.pull-input{flex:1;padding:12px 16px;background:var(--bg-secondary);border:1px solid var(--border);border-radius:10px;color:var(--text-primary);font-family:'JetBrains Mono',monospace;font-size:13px;outline:none;transition:border .2s}
.pull-input:focus{border-color:var(--accent)}
.pull-input::placeholder{color:var(--text-dim)}

/* Progress */
.progress-bar{height:4px;background:var(--bg-secondary);border-radius:2px;overflow:hidden;margin-top:8px}
.progress-fill{height:100%;background:var(--gradient-1);border-radius:2px;transition:width .5s ease;animation:pulse 2s infinite}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:0.7}}

/* Comparison Section */
.compare-grid{display:grid;grid-template-columns:1fr 1fr;gap:24px}
@media(max-width:900px){.compare-grid{grid-template-columns:1fr}}
.compare-card{background:var(--bg-card);border:1px solid var(--border);border-radius:16px;padding:24px;position:relative;overflow:hidden}
.compare-card.holy{border-color:var(--border-glow)}
.compare-card.holy::before{content:'';position:absolute;top:0;left:0;right:0;height:4px;background:var(--gradient-1)}
.compare-card.docker::before{content:'';position:absolute;top:0;left:0;right:0;height:4px;background:linear-gradient(135deg,#2496ed,#066da5)}
.compare-brand{font-size:22px;font-weight:800;margin-bottom:16px;display:flex;align-items:center;gap:10px}
.compare-brand.holy-brand{background:var(--gradient-1);-webkit-background-clip:text;-webkit-text-fill-color:transparent}
.compare-brand.docker-brand{color:#2496ed}
.compare-list{list-style:none;padding:0}
.compare-list li{padding:10px 0;border-bottom:1px solid rgba(30,41,59,0.3);font-size:13px;display:flex;align-items:flex-start;gap:8px;color:var(--text-secondary);line-height:1.5}
.compare-list li::before{content:'';min-width:6px;height:6px;border-radius:50%;margin-top:6px}
.compare-card.holy .compare-list li::before{background:var(--accent-glow)}
.compare-card.docker .compare-list li::before{background:#2496ed}

/* Feature Grid */
.feat-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:16px;margin-top:16px}
.feat{padding:20px;background:rgba(99,102,241,0.04);border:1px solid rgba(99,102,241,0.1);border-radius:12px;transition:all .3s}
.feat:hover{background:rgba(99,102,241,0.08);border-color:var(--border-glow)}
.feat-title{font-size:14px;font-weight:600;margin-bottom:6px;display:flex;align-items:center;gap:6px}
.feat-desc{font-size:12px;color:var(--text-secondary);line-height:1.6}
.feat-code{font-family:'JetBrains Mono',monospace;font-size:11px;padding:8px 12px;background:var(--bg-primary);border-radius:6px;margin-top:8px;color:var(--accent-glow);border:1px solid var(--border)}

/* Tabs content */
.tab-content{display:none}
.tab-content.active{display:block;animation:fadeIn .3s ease}
@keyframes fadeIn{from{opacity:0;transform:translateY(8px)}to{opacity:1;transform:translateY(0)}}

/* Toast */
.toast{position:fixed;bottom:24px;right:24px;padding:14px 20px;border-radius:10px;font-size:13px;font-weight:500;z-index:1000;transform:translateY(100px);transition:transform .3s ease;display:flex;align-items:center;gap:8px}
.toast.show{transform:translateY(0)}
.toast-success{background:rgba(16,185,129,0.95);color:#fff;box-shadow:0 8px 30px rgba(16,185,129,0.3)}
.toast-error{background:rgba(239,68,68,0.95);color:#fff;box-shadow:0 8px 30px rgba(239,68,68,0.3)}

/* Resource Bars */
.res-bar{display:flex;align-items:center;gap:12px;margin:8px 0}
.res-label{min-width:60px;font-size:11px;color:var(--text-dim);font-weight:500}
.res-track{flex:1;height:8px;background:var(--bg-secondary);border-radius:4px;overflow:hidden}
.res-fill{height:100%;border-radius:4px;transition:width .5s}
.res-fill.mem{background:linear-gradient(90deg,var(--accent),var(--accent-glow))}
.res-fill.cpu{background:linear-gradient(90deg,var(--success),#34d399)}
.res-fill.pid{background:linear-gradient(90deg,var(--warning),#fbbf24)}
.res-value{min-width:65px;text-align:right;font-size:11px;font-family:'JetBrains Mono',monospace;color:var(--text-secondary)}

/* Empty state */  
.empty{text-align:center;padding:48px 24px;color:var(--text-dim)}
.empty-icon{font-size:48px;margin-bottom:12px;opacity:0.5}
.empty-text{font-size:14px;margin-bottom:16px}
</style>
</head>
<body>

<div class="header">
  <div class="logo">
    <div class="logo-icon">HC</div>
    <div class="logo-text">HolyContainer</div>
    <span class="logo-version">v2.0.0</span>
  </div>
  <div class="header-actions">
    <span id="platform-badge" class="badge badge-created">...</span>
    <span id="whp-badge" class="badge" style="display:none">WHP ✓</span>
  </div>
</div>

<div class="nav">
  <button class="nav-btn active" onclick="showTab('dashboard')">📊 Dashboard</button>
  <button class="nav-btn" onclick="showTab('containers')">📦 Containers</button>
  <button class="nav-btn" onclick="showTab('images')">💿 Images</button>
  <button class="nav-btn" onclick="showTab('compare')">⚡ Docker vs Holy</button>
  <button class="nav-btn" onclick="showTab('tech')">🔧 Technology</button>
</div>

<div class="main">

<!-- ═══ DASHBOARD TAB ═══ -->
<div id="tab-dashboard" class="tab-content active">
  <div class="stats">
    <div class="stat">
      <div class="stat-label">Containers</div>
      <div class="stat-value" id="stat-containers">—</div>
      <div class="stat-detail" id="stat-containers-detail">Loading...</div>
    </div>
    <div class="stat">
      <div class="stat-label">Images</div>
      <div class="stat-value" id="stat-images">—</div>
      <div class="stat-detail" id="stat-images-detail">Loading...</div>
    </div>
    <div class="stat">
      <div class="stat-label">Platform</div>
      <div class="stat-value" id="stat-platform" style="font-size:20px">—</div>
      <div class="stat-detail" id="stat-platform-detail">Loading...</div>
    </div>
    <div class="stat">
      <div class="stat-label">Dependencies</div>
      <div class="stat-value" style="background:var(--gradient-1);-webkit-background-clip:text;-webkit-text-fill-color:transparent">0</div>
      <div class="stat-detail">Pure Rust, zero crates</div>
    </div>
  </div>

  <div class="card">
    <div class="card-header">
      <div class="card-title"><span>🚀</span> Quick Pull</div>
    </div>
    <div class="pull-bar">
      <input type="text" class="pull-input" id="quick-pull" placeholder="Image name  (e.g.  ubuntu:22.04,  alpine:latest,  nginx:latest)" onkeydown="if(event.key==='Enter')pullImage()">
      <button class="btn btn-primary" onclick="pullImage()">⬇ Pull Image</button>
    </div>
    <div id="pull-progress" style="display:none">
      <div style="font-size:12px;color:var(--text-secondary)" id="pull-status">Pulling...</div>
      <div class="progress-bar"><div class="progress-fill" style="width:60%"></div></div>
    </div>
  </div>

  <div class="card">
    <div class="card-header">
      <div class="card-title"><span>📦</span> Recent Containers</div>
    </div>
    <div id="dashboard-containers" class="table-wrap"></div>
  </div>
</div>

<!-- ═══ CONTAINERS TAB ═══ -->
<div id="tab-containers" class="tab-content">
  <div class="card">
    <div class="card-header">
      <div class="card-title"><span>📦</span> All Containers</div>
      <button class="btn btn-primary btn-sm" onclick="refreshContainers()">↻ Refresh</button>
    </div>
    <div id="containers-list" class="table-wrap"></div>
  </div>
</div>

<!-- ═══ IMAGES TAB ═══ -->
<div id="tab-images" class="tab-content">
  <div class="card">
    <div class="card-header">
      <div class="card-title"><span>💿</span> Pull Image from Docker Hub</div>
    </div>
    <div class="pull-bar">
      <input type="text" class="pull-input" id="images-pull" placeholder="e.g. ubuntu:22.04, alpine:latest, nginx:1.25, node:20-alpine" onkeydown="if(event.key==='Enter')pullFromImagesTab()">
      <button class="btn btn-primary" onclick="pullFromImagesTab()">⬇ Pull</button>
    </div>
    <p style="font-size:12px;color:var(--text-dim);margin-bottom:20px">
      Popular images: 
      <a href="#" style="color:var(--accent)" onclick="document.getElementById('images-pull').value='ubuntu:22.04'">ubuntu:22.04</a> · 
      <a href="#" style="color:var(--accent)" onclick="document.getElementById('images-pull').value='alpine:latest'">alpine:latest</a> · 
      <a href="#" style="color:var(--accent)" onclick="document.getElementById('images-pull').value='nginx:latest'">nginx:latest</a> · 
      <a href="#" style="color:var(--accent)" onclick="document.getElementById('images-pull').value='node:20-alpine'">node:20-alpine</a> · 
      <a href="#" style="color:var(--accent)" onclick="document.getElementById('images-pull').value='python:3.12-slim'">python:3.12</a> · 
      <a href="#" style="color:var(--accent)" onclick="document.getElementById('images-pull').value='postgres:16'">postgres:16</a>
    </p>
  </div>
  <div class="card">
    <div class="card-header">
      <div class="card-title"><span>📂</span> Local Images</div>
      <button class="btn btn-primary btn-sm" onclick="refreshImages()">↻ Refresh</button>
    </div>
    <div id="images-list" class="table-wrap"></div>
  </div>
</div>

<!-- ═══ COMPARISON TAB ═══ -->
<div id="tab-compare" class="tab-content">
  <h2 style="font-size:24px;font-weight:800;margin-bottom:8px;background:var(--gradient-1);-webkit-background-clip:text;-webkit-text-fill-color:transparent;display:inline-block">Docker vs HolyContainer</h2>
  <p style="color:var(--text-secondary);margin-bottom:24px;font-size:14px">
    A side-by-side breakdown of how HolyContainer differs from Docker at every layer.
  </p>

  <div class="compare-grid">
    <div class="compare-card holy">
      <div class="compare-brand holy-brand">⚡ HolyContainer</div>
      <ul class="compare-list">
        <li><strong>Zero dependencies.</strong> Every component — HTTP, JSON, gzip, tar, hypervisor — is hand-written in pure Rust. Cargo.toml has literally zero crates.</li>
        <li><strong>Single binary.</strong> One executable (~2 MB). No daemon, no containerd, no runc, no shim. Just the binary.</li>
        <li><strong>Transparent hypervisor.</strong> On Windows, boots Linux kernels via WHP with page tables and GDT you can read and audit. Every CPU register, every memory mapping is explicit in the code.</li>
        <li><strong>No black boxes.</strong> You can trace every syscall from CLI to kernel. The HTTP client, JSON parser, and DEFLATE decompressor are all readable, auditable, 300-line modules.</li>
        <li><strong>True isolation depth.</strong> Uses the same kernel primitives as Docker (namespaces, cgroups, seccomp) but the BPF filter bytecode is hand-assembled — you control every blocked syscall.</li>
        <li><strong>Provably minimal attack surface.</strong> No third-party code means no supply chain risk. Zero transitive dependencies to audit.</li>
        <li><strong>Compose-like orchestration.</strong> Built-in multi-container stacks with dependency ordering. No separate tool needed.</li>
      </ul>
    </div>
    <div class="compare-card docker">
      <div class="compare-brand docker-brand">🐳 Docker</div>
      <ul class="compare-list">
        <li><strong>~400+ dependencies</strong> (Go modules). Each is a potential supply chain attack vector. Comprehensive but complex.</li>
        <li><strong>Multi-process architecture.</strong> Docker CLI → dockerd daemon → containerd → runc → shim → container. Multiple services to manage and secure.</li>
        <li><strong>Black-box VM on Windows.</strong> Uses WSL2 (a full Hyper-V Linux VM) that you can't inspect or customize. Microsoft controls the kernel.</li>
        <li><strong>Mature ecosystem.</strong> 10+ years of production hardening. Docker Hub is the world's largest container registry. Extensive tooling and community.</li>
        <li><strong>Industry standard.</strong> OCI-compliant, works with Kubernetes, Swarm, ECS, and every CI/CD system. The de facto container runtime.</li>
        <li><strong>Comprehensive security.</strong> AppArmor, SELinux, user namespaces, rootless mode, content trust, image signing. Battle-tested in production.</li>
        <li><strong>Docker Compose.</strong> Full YAML-based multi-container orchestration with volumes, networks, and health checks.</li>
      </ul>
    </div>
  </div>

  <div class="card" style="margin-top:24px">
    <div class="card-header">
      <div class="card-title"><span>📊</span> Technical Comparison</div>
    </div>
    <div class="table-wrap">
      <table>
        <tr><th>Aspect</th><th style="color:var(--accent-glow)">HolyContainer</th><th style="color:#2496ed">Docker</th></tr>
        <tr><td>Language</td><td>Pure Rust</td><td>Go</td></tr>
        <tr><td>External dependencies</td><td style="color:var(--success);font-weight:600">0</td><td>~400+ Go modules</td></tr>
        <tr><td>Architecture</td><td>Single binary</td><td>CLI + daemon + containerd + runc</td></tr>
        <tr><td>Linux on Windows</td><td>Hand-written WHP hypervisor</td><td>WSL2 (Hyper-V VM)</td></tr>
        <tr><td>HTTP client</td><td>Hand-written (WinHTTP FFI / raw sockets)</td><td>Go net/http</td></tr>
        <tr><td>JSON parsing</td><td>Hand-written recursive descent parser</td><td>encoding/json</td></tr>
        <tr><td>Compression</td><td>Hand-written RFC 1951 DEFLATE</td><td>compress/gzip</td></tr>
        <tr><td>Tar handling</td><td>Hand-written UStar reader/writer</td><td>archive/tar</td></tr>
        <tr><td>Seccomp</td><td>Hand-assembled BPF bytecode</td><td>libseccomp bindings</td></tr>
        <tr><td>Container isolation</td><td>Direct syscalls (clone, pivot_root, prctl)</td><td>Via runc (also syscalls)</td></tr>
        <tr><td>Image format</td><td>OCI / Docker v2 manifests</td><td>OCI / Docker v2</td></tr>
        <tr><td>Registry protocol</td><td>Docker Registry HTTP API v2</td><td>Docker Registry HTTP API v2</td></tr>
        <tr><td>Orchestration</td><td>Built-in TOML stacks</td><td>Docker Compose (separate tool)</td></tr>
        <tr><td>Supply chain risk</td><td style="color:var(--success)">Zero (no third-party code)</td><td>Moderate (hundreds of deps)</td></tr>
      </table>
    </div>
  </div>

  <div class="card" style="margin-top:20px">
    <div class="card-header">
      <div class="card-title"><span>🛡️</span> Security: Why Zero Dependencies Matters</div>
    </div>
    <div style="font-size:13px;color:var(--text-secondary);line-height:1.8">
      <p style="margin-bottom:12px">Every third-party dependency is a trust decision. When Docker imports 400+ Go modules, each module author (and their dependencies' authors) becomes part of the trusted computing base. A single compromised package in that chain can:</p>
      <div class="feat-grid">
        <div class="feat">
          <div class="feat-title">🔒 Supply Chain Attack</div>
          <div class="feat-desc">A malicious update to any transitive dependency can inject code into the container runtime itself. See: event-stream, ua-parser-js, colors.js incidents.</div>
        </div>
        <div class="feat">
          <div class="feat-title">🔍 Audit Complexity</div>
          <div class="feat-desc">Auditing 400+ modules is impractical. HolyContainer's ~4,000 lines of Rust can be fully audited in a day by a single engineer.</div>
        </div>
        <div class="feat">
          <div class="feat-title">🎯 Minimal Attack Surface</div>
          <div class="feat-desc">Less code = fewer bugs. HolyContainer's hand-written HTTP client has exactly the features needed for registry communication, nothing extra.</div>
        </div>
        <div class="feat">
          <div class="feat-title">🧠 Full Understanding</div>
          <div class="feat-desc">Every byte decompressed, every syscall made, every page table entry — the developer understands and controls it. No magic, no abstraction layers hiding behavior.</div>
        </div>
      </div>
    </div>
  </div>
</div>

<!-- ═══ TECHNOLOGY TAB ═══ -->
<div id="tab-tech" class="tab-content">
  <h2 style="font-size:24px;font-weight:800;margin-bottom:8px;background:var(--gradient-2);-webkit-background-clip:text;-webkit-text-fill-color:transparent;display:inline-block">Under the Hood</h2>
  <p style="color:var(--text-secondary);margin-bottom:24px;font-size:14px">
    Every component that other projects import as a library, we wrote from scratch.
  </p>

  <div class="feat-grid">
    <div class="feat">
      <div class="feat-title">🌐 HTTP Client</div>
      <div class="feat-desc">On Windows: Hand-declared WinHTTP FFI bindings (WinHttpOpen → WinHttpConnect → WinHttpSendRequest). On Linux: raw TCP sockets for HTTP, system TLS for HTTPS. Handles redirects, chunked encoding, bearer auth.</div>
      <div class="feat-code">WinHttpOpen() → WinHttpConnect() → WinHttpSendRequest() → WinHttpReceiveResponse()</div>
    </div>
    <div class="feat">
      <div class="feat-title">📝 JSON Parser</div>
      <div class="feat-desc">Recursive descent parser handling all JSON types: objects, arrays, strings (with Unicode surrogate pairs), numbers, booleans, null. Used for Docker Registry API responses.</div>
      <div class="feat-code">parse() → parse_value() → parse_object() → parse_string() → parse_unicode_escape()</div>
    </div>
    <div class="feat">
      <div class="feat-title">🗜️ DEFLATE Decompressor</div>
      <div class="feat-desc">Full RFC 1951 implementation: Huffman tree building (fixed + dynamic), LZ77 back-reference decoding, sliding window. RFC 1952 gzip wrapper with CRC32 validation.</div>
      <div class="feat-code">inflate() → decode_huffman_codes() → read_bits() → lz77_copy(distance, length)</div>
    </div>
    <div class="feat">
      <div class="feat-title">🖥️ WHP Hypervisor</div>
      <div class="feat-desc">Creates x86_64 long-mode VMs via Windows Hypervisor Platform. Sets up 4-level page tables (PML4→PDPT→PD), GDT, loads Linux bzImage kernels, handles VM exits (I/O, MSR, CPUID, HLT).</div>
      <div class="feat-code">WHvCreatePartition() → setup_page_tables() → load_kernel() → WHvRunVirtualProcessor()</div>
    </div>
    <div class="feat">
      <div class="feat-title">📦 Docker Registry Client</div>
      <div class="feat-desc">Docker Registry HTTP API v2: bearer token auth discovery, manifest list resolution (multi-arch → amd64/linux), layer blob download with cross-origin redirect handling.</div>
      <div class="feat-code">auth.docker.io/token → /v2/.../manifests/tag → /v2/.../blobs/sha256:...</div>
    </div>
    <div class="feat">
      <div class="feat-title">📁 Tar Reader/Writer</div>
      <div class="feat-desc">UStar format tar reader/writer. Handles regular files, directories, symlinks, hard links. Processes whiteout files (.wh.*) for overlay filesystem layer semantics.</div>
      <div class="feat-code">parse_header() → extract_file() → handle_whiteout() → apply_permissions()</div>
    </div>
    <div class="feat">
      <div class="feat-title">🔒 Seccomp BPF</div>
      <div class="feat-desc">Hand-assembled Berkeley Packet Filter bytecode for syscall filtering. Blocks dangerous syscalls (reboot, kexec, mount, etc.) while allowing normal operation.</div>
      <div class="feat-code">BPF_STMT(BPF_LD, syscall_nr) → BPF_JUMP(BPF_JEQ, blocked) → BPF_RET(ALLOW/KILL)</div>
    </div>
    <div class="feat">
      <div class="feat-title">🔌 Virtio Devices</div>
      <div class="feat-desc">Virtio console (serial I/O bridging VM↔host), block device (rootfs disk image with sector read/write), and network device (packet injection/capture).</div>
      <div class="feat-code">VirtioConsole::handle_output() → VirtioBlock::read_sectors() → VirtioNet::inject_packet()</div>
    </div>
  </div>

  <div class="card" style="margin-top:24px">
    <div class="card-header">
      <div class="card-title"><span>📐</span> Architecture</div>
    </div>
    <div style="font-family:'JetBrains Mono',monospace;font-size:12px;color:var(--text-secondary);line-height:1.8;padding:16px;background:var(--bg-primary);border-radius:10px;border:1px solid var(--border);white-space:pre;overflow-x:auto">
                          holycontainer binary
                    ┌──────────────┴──────────────┐
                 main.rs                       compose.rs
              (CLI parser)                  (stack orchestrator)
                    │
              container.rs ──── config.rs ──── image.rs
           (lifecycle mgmt)   (serialization)   (tar r/w)
                    │
              platform/mod.rs
           ┌────────┴────────┐
       linux/              windows/
    ┌────┼────┐        ┌────┼────┐
  syscall  cgroup   winapi  job   whp.rs
  namespace seccomp  sandbox      vmm.rs
  capabilities       filesystem   virtio.rs
  filesystem          network
  network
                    │
        ┌───────────┼───────────┐
     json.rs     http.rs     gzip.rs
   (parser)    (WinHTTP/     (DEFLATE)
              raw sockets)
                    │
              registry.rs
         (Docker Registry v2)</div>
  </div>
</div>

</div><!-- /main -->

<div class="toast" id="toast"></div>

<script>
// ─── State ──────────────────────────────────────────────────────────────
let containers = [], images = [], sysInfo = {};

// ─── API ────────────────────────────────────────────────────────────────
async function api(path, method = 'GET') {
  try {
    const r = await fetch(path, { method });
    return await r.json();
  } catch(e) {
    console.error('API error:', e);
    return null;
  }
}

// ─── Data Loading ───────────────────────────────────────────────────────
async function loadAll() {
  [containers, images, sysInfo] = await Promise.all([
    api('/api/containers'),
    api('/api/images'),
    api('/api/system')
  ]);
  containers = containers || [];
  images = images || [];
  sysInfo = sysInfo || {};
  render();
}

// ─── Render ─────────────────────────────────────────────────────────────
function render() {
  // Stats
  const running = containers.filter(c => c.state === 'running').length;
  document.getElementById('stat-containers').textContent = containers.length;
  document.getElementById('stat-containers-detail').textContent = `${running} running`;
  document.getElementById('stat-images').textContent = images.length;
  document.getElementById('stat-images-detail').textContent = images.map(i => i.name).join(', ') || 'None pulled';
  document.getElementById('stat-platform').textContent = sysInfo.platform || '—';
  document.getElementById('stat-platform-detail').textContent = sysInfo.whp_available ? 'WHP ✓ VM Ready' : 'Native mode';
  document.getElementById('platform-badge').textContent = sysInfo.platform || '...';
  
  if (sysInfo.whp_available) {
    const whp = document.getElementById('whp-badge');
    whp.style.display = 'inline-block';
    whp.className = 'badge badge-running';
  }

  // Container tables
  const ctHtml = renderContainerTable(containers);
  document.getElementById('dashboard-containers').innerHTML = ctHtml;
  document.getElementById('containers-list').innerHTML = ctHtml;

  // Images table
  document.getElementById('images-list').innerHTML = renderImagesTable(images);
}

function renderContainerTable(list) {
  if (!list.length) return '<div class="empty"><div class="empty-icon">📦</div><div class="empty-text">No containers yet</div><div style="font-size:12px;color:var(--text-dim)">Use <code>holycontainer create</code> or pull an image above</div></div>';
  
  let h = '<table><tr><th>Name</th><th>State</th><th>Resources</th><th>Hostname</th><th>Actions</th></tr>';
  for (const c of list) {
    const mem = formatBytes(c.memory);
    const badge = c.state === 'running' ? 'badge-running' : c.state === 'created' ? 'badge-created' : 'badge-stopped';
    h += `<tr>
      <td style="font-weight:600;color:var(--text-primary)">${esc(c.name)}</td>
      <td><span class="badge ${badge}">${c.state}</span></td>
      <td>
        <div class="res-bar"><span class="res-label">MEM</span><div class="res-track"><div class="res-fill mem" style="width:${c.memory ? 60 : 0}%"></div></div><span class="res-value">${mem}</span></div>
        <div class="res-bar"><span class="res-label">CPU</span><div class="res-track"><div class="res-fill cpu" style="width:${c.cpu || 0}%"></div></div><span class="res-value">${c.cpu || 0}%</span></div>
        <div class="res-bar"><span class="res-label">PIDs</span><div class="res-track"><div class="res-fill pid" style="width:${c.pids ? Math.min(c.pids * 1.5, 100) : 0}%"></div></div><span class="res-value">${c.pids || '∞'}</span></div>
      </td>
      <td style="font-family:'JetBrains Mono',monospace;font-size:12px">${esc(c.hostname)}</td>
      <td>
        ${c.state === 'running' ? `<button class="btn btn-danger btn-sm" onclick="stopContainer('${esc(c.name)}')">⬛ Stop</button>` : ''}
        ${c.state !== 'running' ? `<button class="btn btn-danger btn-sm" onclick="rmContainer('${esc(c.name)}')">🗑 Remove</button>` : ''}
      </td>
    </tr>`;
  }
  return h + '</table>';
}

function renderImagesTable(list) {
  if (!list.length) return '<div class="empty"><div class="empty-icon">💿</div><div class="empty-text">No images pulled yet</div><div style="font-size:12px;color:var(--text-dim)">Pull one above — try <code>ubuntu:22.04</code> or <code>alpine:latest</code></div></div>';
  
  let h = '<table><tr><th>Image</th><th>Layers</th><th>Size</th><th>Path</th></tr>';
  for (const img of list) {
    h += `<tr>
      <td style="font-weight:600;color:var(--text-primary)">${esc(img.name)}</td>
      <td>${img.layers}</td>
      <td>${formatBytes(img.size)}</td>
      <td style="font-family:'JetBrains Mono',monospace;font-size:11px;color:var(--text-dim);max-width:300px;overflow:hidden;text-overflow:ellipsis">${esc(img.path)}</td>
    </tr>`;
  }
  return h + '</table>';
}

// ─── Actions ────────────────────────────────────────────────────────────
async function pullImage() {
  const input = document.getElementById('quick-pull');
  const image = input.value.trim();
  if (!image) return;
  
  document.getElementById('pull-progress').style.display = 'block';
  document.getElementById('pull-status').textContent = `Pulling ${image}...`;
  toast('Pulling ' + image + '...', 'success');
  
  const result = await api('/api/pull/' + encodeURIComponent(image), 'POST');
  document.getElementById('pull-progress').style.display = 'none';
  
  if (result && result.success) {
    toast('✓ ' + image + ' pulled successfully!', 'success');
    input.value = '';
    loadAll();
  } else {
    toast('✗ Pull failed: ' + (result?.error || 'unknown error'), 'error');
  }
}

function pullFromImagesTab() {
  const v = document.getElementById('images-pull').value.trim();
  if (v) {
    document.getElementById('quick-pull').value = v;
    pullImage();
  }
}

async function stopContainer(name) {
  const r = await api('/api/stop/' + name, 'POST');
  toast(r?.success ? '⬛ Stopped ' + name : '✗ ' + (r?.error || 'failed'), r?.success ? 'success' : 'error');
  loadAll();
}

async function rmContainer(name) {
  const r = await api('/api/rm/' + name, 'POST');
  toast(r?.success ? '🗑 Removed ' + name : '✗ ' + (r?.error || 'failed'), r?.success ? 'success' : 'error');
  loadAll();
}

async function refreshContainers() { await loadAll(); toast('↻ Refreshed', 'success'); }
async function refreshImages() { await loadAll(); toast('↻ Refreshed', 'success'); }

// ─── Tabs ───────────────────────────────────────────────────────────────
function showTab(name) {
  document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
  document.querySelectorAll('.nav-btn').forEach(b => b.classList.remove('active'));
  document.getElementById('tab-' + name).classList.add('active');
  event.target.classList.add('active');
}

// ─── Helpers ────────────────────────────────────────────────────────────
function formatBytes(b) {
  if (!b || b === 0) return '—';
  if (b < 1024) return b + ' B';
  if (b < 1048576) return (b/1024).toFixed(0) + ' KB';
  if (b < 1073741824) return (b/1048576).toFixed(1) + ' MB';
  return (b/1073741824).toFixed(2) + ' GB';
}

function esc(s) { return String(s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;'); }

function toast(msg, type='success') {
  const t = document.getElementById('toast');
  t.textContent = msg;
  t.className = 'toast toast-' + type + ' show';
  setTimeout(() => t.classList.remove('show'), 3000);
}

// ─── Init ───────────────────────────────────────────────────────────────
loadAll();
setInterval(loadAll, 10000);
</script>
</body>
</html>
"##;
