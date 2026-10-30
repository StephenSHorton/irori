//! The mcpserver test rig (I:internal/mcpserver/fakecore_test.go): a fake
//! Core serving the routes_v02.go contract over HTTP on 127.0.0.1, and the
//! server driven over in-memory pipes.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mcp::{Core, HttpCore, Options, Server};

pub const FAKE_TOKEN: &str = "test-token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCall {
    pub name: String,
    pub member: String,
    pub body: String,
}

#[derive(Default)]
pub struct FakeState {
    pub token: String,
    pub runs: usize,
    pub fail_join: usize,
    /// `None` answers 404, like a v0.1 Core.
    pub specs: Option<String>,
    pub calls: Vec<FakeCall>,
    pub joins: Vec<(String, String)>,
    pub secrets: HashMap<String, String>,
    pub keys: HashMap<String, String>,
    pub n: usize,
    pub attaches: Vec<String>,
}

pub struct FakeCore {
    pub addr: String,
    pub st: Arc<Mutex<FakeState>>,
    pub stream_on: Receiver<String>,
    events: Arc<Mutex<Option<Sender<String>>>>,
}

/// The specs mcpserver_test.go's testSpecs gives, as the Core's JSON.
pub fn test_specs() -> String {
    r#"[{"name":"agents","description":"list agents","inputSchema":{"type":"object","properties":{"status":{"type":"string","description":"marker-status"}}},"readOnly":true,"clients":["claude","grok"]},{"name":"send","description":"send a message","inputSchema":{"type":"object","properties":{"to":{"type":"string"},"text":{"type":"string"}},"required":["text"]},"readOnly":false,"clients":["claude","grok"]},{"name":"claude_only","description":"mod only","inputSchema":{"type":"object"},"readOnly":false,"clients":["claude"]},{"name":"bad-name","description":"invalid","inputSchema":{"type":"object"},"readOnly":false,"clients":["grok"]},{"name":"noschema","description":"invalid","inputSchema":{"type":"string"},"readOnly":false,"clients":["grok"]}]"#.to_string()
}

fn write_json(s: &mut TcpStream, code: u16, body: &str) {
    let body = format!("{body}\n");
    let _ = write!(
        s,
        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn err_json(s: &mut TcpStream, code: u16, msg: &str) {
    write_json(s, code, &format!(r#"{{"error":"{msg}"}}"#));
}

fn chunk(s: &mut TcpStream, data: &str) -> std::io::Result<()> {
    write!(s, "{:x}\r\n{data}\r\n", data.len())?;
    s.flush()
}

impl FakeCore {
    pub fn new(specs: Option<String>) -> FakeCore {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let st = Arc::new(Mutex::new(FakeState {
            token: FAKE_TOKEN.into(),
            specs,
            ..FakeState::default()
        }));
        let (on_tx, on_rx) = channel();
        let events: Arc<Mutex<Option<Sender<String>>>> = Arc::default();
        let (st2, ev2) = (st.clone(), events.clone());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(conn) = conn else { break };
                let (st, ev, on) = (st2.clone(), ev2.clone(), on_tx.clone());
                std::thread::spawn(move || serve(conn, st, ev, on));
            }
        });
        FakeCore {
            addr,
            st,
            stream_on: on_rx,
            events,
        }
    }

    pub fn dial(&self) -> mcp::Dialer {
        let (addr, st) = (self.addr.clone(), self.st.clone());
        Arc::new(move || {
            let token = st.lock().unwrap().token.clone();
            Ok(Arc::new(HttpCore::new(&format!("http://{addr}"), &token)) as Arc<dyn Core>)
        })
    }

    /// A restarted Core: every secret gone, a new token.
    pub fn forget(&self) {
        let mut s = self.st.lock().unwrap();
        s.secrets.clear();
        s.runs += 1;
        s.token = format!("{FAKE_TOKEN}-{}", s.runs);
    }

    /// Another `irori mcp` joined the same session: `secret` is retired.
    pub fn takeover(&self, secret: &str) {
        let mut s = self.st.lock().unwrap();
        let key = s.secrets.remove(secret).expect("live secret");
        s.secrets.insert(format!("other-{secret}"), key);
    }

    /// Feeds one event line to the open stream.
    pub fn event(&self, line: &str) {
        let tx = self.events.lock().unwrap().clone().expect("no stream open");
        tx.send(line.to_string()).unwrap();
    }

    /// Ends the open stream.
    pub fn cut(&self) {
        if let Some(tx) = self.events.lock().unwrap().take() {
            let _ = tx.send(String::new());
        }
    }

    pub fn wait_stream(&self) -> String {
        self.stream_on
            .recv_timeout(Duration::from_secs(5))
            .expect("member stream never opened")
    }

    pub fn snapshot(&self) -> (Vec<FakeCall>, Vec<(String, String)>) {
        let s = self.st.lock().unwrap();
        (s.calls.clone(), s.joins.clone())
    }
}

fn query_get(q: &str, k: &str) -> String {
    for kv in q.split('&') {
        if let Some((a, b)) = kv.split_once('=')
            && a == k
        {
            return b.to_string();
        }
    }
    String::new()
}

fn serve(
    mut s: TcpStream,
    st: Arc<Mutex<FakeState>>,
    events: Arc<Mutex<Option<Sender<String>>>>,
    on: Sender<String>,
) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    let _ = r.read_exact(&mut body);
    let body = String::from_utf8_lossy(&body).to_string();
    let token = st.lock().unwrap().token.clone();
    if headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {token}")) {
        return err_json(&mut s, 401, "bad token");
    }
    let member = headers.get("x-irori-member").cloned().unwrap_or_default();
    let known = st.lock().unwrap().secrets.contains_key(&member);
    if method == "GET" && path == "/v1/tools" {
        if query_get(query, "client") != "grok" {
            return err_json(&mut s, 400, "client");
        }
        let specs = st.lock().unwrap().specs.clone();
        match specs {
            None => err_json(&mut s, 404, "not found"),
            Some(j) => write_json(&mut s, 200, &j),
        }
    } else if method == "POST" && path.starts_with("/v1/tools/") {
        let name = path.trim_start_matches("/v1/tools/").to_string();
        st.lock().unwrap().calls.push(FakeCall {
            name: name.clone(),
            member: member.clone(),
            body,
        });
        if !member.is_empty() && !known {
            return err_json(&mut s, 403, "unknown member");
        }
        write_json(&mut s, 200, &format!(r#"{{"text":"ok {name}"}}"#));
    } else if method == "POST" && path == "/v1/members" {
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let name = v["name"].as_str().unwrap_or("").to_string();
        let session = v["session"].as_str().unwrap_or("").to_string();
        let mut f = st.lock().unwrap();
        if f.fail_join > 0 {
            f.fail_join -= 1;
            drop(f);
            return err_json(&mut s, 500, "warming up");
        }
        f.joins.push((name.clone(), session.clone()));
        f.n += 1;
        let n = f.n;
        let key = f
            .keys
            .entry(session)
            .or_insert_with(|| format!("dev1/grok-{n}"))
            .clone();
        let secret = format!("secret-{n}");
        f.secrets.insert(secret.clone(), key.clone());
        drop(f);
        write_json(
            &mut s,
            200,
            &serde_json::json!({"memberKey": key, "memberSecret": secret, "name": name})
                .to_string(),
        );
    } else if method == "GET" && path == "/v1/attach" {
        let secret = query_get(query, "member");
        let key = {
            let mut f = st.lock().unwrap();
            f.attaches.push(secret.clone());
            f.secrets.get(&secret).cloned()
        };
        let Some(key) = key else {
            return err_json(&mut s, 403, "unknown member");
        };
        let _ = write!(
            s,
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\n\r\n"
        );
        if chunk(
            &mut s,
            &format!("{{\"type\":\"hello\",\"v\":1,\"agentKey\":\"{key}\"}}\n"),
        )
        .is_err()
        {
            return;
        }
        let (tx, rx) = channel::<String>();
        *events.lock().unwrap() = Some(tx);
        let _ = on.send(secret);
        while let Ok(ev) = rx.recv() {
            if ev.is_empty() {
                break; // cut
            }
            if chunk(&mut s, "\n").is_err() || chunk(&mut s, &format!("{ev}\n")).is_err() {
                break;
            }
        }
        let _ = write!(s, "0\r\n\r\n");
    } else {
        err_json(&mut s, 404, "no route");
    }
}

/// The server over in-memory pipes.
pub struct Wire {
    tx: Option<Sender<Vec<u8>>>,
    pub rx: Receiver<String>,
    pub srv: Server,
    done: Option<std::thread::JoinHandle<()>>,
}

struct PipeIn {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for PipeIn {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.fill_buf()?.len().min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for PipeIn {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Err(_) => return Ok(&[]),
            }
        }
        Ok(&self.buf[self.pos..])
    }
    fn consume(&mut self, n: usize) {
        self.pos += n;
    }
}

struct PipeOut {
    tx: Sender<String>,
    buf: Vec<u8>,
}

impl Write for PipeOut {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        while let Some(i) = self.buf.iter().position(|&c| c == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let _ = self
                .tx
                .send(String::from_utf8_lossy(&line[..line.len() - 1]).to_string());
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn quick(dial: Option<mcp::Dialer>) -> Options {
    Options {
        dial,
        version: "1.2.3".into(),
        min_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(40),
        ..Options::default()
    }
}

impl Wire {
    pub fn start(opts: Options) -> Wire {
        let srv = Server::new(opts);
        let (in_tx, in_rx) = channel();
        let (out_tx, out_rx) = channel();
        let s2 = srv.clone();
        let done = std::thread::spawn(move || {
            let input = PipeIn {
                rx: in_rx,
                buf: vec![],
                pos: 0,
            };
            s2.run(
                input,
                PipeOut {
                    tx: out_tx,
                    buf: vec![],
                },
            )
            .unwrap();
        });
        Wire {
            tx: Some(in_tx),
            rx: out_rx,
            srv,
            done: Some(done),
        }
    }

    pub fn send(&self, line: &str) {
        self.tx
            .as_ref()
            .unwrap()
            .send(format!("{line}\n").into_bytes())
            .unwrap();
    }

    pub fn recv(&self) -> String {
        self.rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no answer from the server")
    }

    pub fn no_recv(&self, d: Duration) {
        if let Ok(l) = self.rx.recv_timeout(d) {
            panic!("unexpected line {l}");
        }
    }

    /// Sends a request and returns its answer.
    pub fn ask(&self, line: &str) -> String {
        self.send(line);
        self.recv()
    }

    /// Calls a tool; (text, isError).
    pub fn call(&self, id: u64, name: &str, args: &str, meta: Option<&str>) -> (String, bool) {
        let meta = meta.map(|m| format!(r#","_meta":{m}"#)).unwrap_or_default();
        let out = self.ask(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{args}{meta}}}}}"#
        ));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let r = &v["result"];
        (
            r["content"][0]["text"].as_str().unwrap_or("").to_string(),
            r["isError"].as_bool().unwrap_or(false),
        )
    }

    pub fn init(&self) {
        self.ask(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#);
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#);
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(d) = self.done.take() {
            let _ = d.join();
        }
    }
}
