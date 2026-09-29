//! Loopback HTTP/1.1 stub for tests: every connection gets one response from `handler`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    /// Path including the query string.
    pub target: String,
    pub authorization: Option<String>,
    pub body: String,
}

pub type Log = Arc<Mutex<Vec<Seen>>>;

/// Starts the stub; returns its base URL and the request log. The thread lives until the test
/// process ends.
pub fn serve<F>(handler: F) -> (String, Log)
where
    F: Fn(&Seen) -> (u16, String) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind: {e}"));
    let url = format!("http://127.0.0.1:{}", listener.local_addr().map(|a| a.port()).unwrap_or_default());
    let log: Log = Arc::default();
    let thread_log = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some(seen) = read_request(&mut stream) else { continue };
            let (status, body) = handler(&seen);
            if let Ok(mut l) = thread_log.lock() {
                l.push(seen);
            }
            let res = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(res.as_bytes());
        }
    });
    (url, log)
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let header = |name: &str| {
        head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    };
    let len = header("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    while buf.len() < head_end + len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let mut first = head.lines().next().unwrap_or_default().split(' ');
    Some(Seen {
        method: first.next().unwrap_or_default().to_owned(),
        target: first.next().unwrap_or_default().to_owned(),
        authorization: header("authorization"),
        body: String::from_utf8_lossy(&buf[head_end..buf.len().min(head_end + len)]).to_string(),
    })
}

/// A session response for an API token bound to `demo/local-demo`.
pub fn session_json() -> String {
    serde_json::json!({
        "user": { "id": "u", "email": "a@example.test", "display_name": "A" },
        "projects": [{ "id": "demo", "tenant_id": "local", "name": "Demo", "role": "viewer",
            "environments": [{ "id": "local-demo", "name": "local-demo", "is_demo": true }] }]
    })
    .to_string()
}
