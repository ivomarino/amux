//! `amux-rs` — the Rust CLI (Phase 8). Ships beside the bash `amux` until
//! Phase 11 cutover renames it; verbs mirror the bash script's core surface
//! so muscle memory transfers.
//!
//! Talks to the RUST server (default https://localhost:8823) with the
//! shared bearer token from ~/.amux/auth-token. Gate 409s are surfaced
//! LOUDLY with the exact retry command (the AMUX-2325 lesson: the sanctioned
//! escape must be walkable from the sanctioned tool, or agents hand-roll
//! curl and lose attribution).

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "amux-rs", version, about = "amux command-line interface (Rust server)")]
struct Cli {
    /// Server base URL.
    #[arg(long, env = "AMUX_RS_URL", default_value = "https://localhost:8823")]
    url: String,
    /// Session/worker name stamped as X-Amux-Session on mutations.
    #[arg(long, env = "AMUX_SESSION")]
    session: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Board operations.
    Board {
        #[command(subcommand)]
        cmd: BoardCmd,
    },
    /// Worker operations.
    Workers {
        #[command(subcommand)]
        cmd: WorkerCmd,
    },
    /// Send a message to a worker.
    Send {
        worker: String,
        /// Message text; reads stdin when omitted (fleet convention: --stdin
        /// semantics — shell interpolation never touches piped bytes).
        text: Option<String>,
    },
    /// Schedule operations.
    Schedules {
        #[command(subcommand)]
        cmd: SchedCmd,
    },
    /// Server health.
    Health,
}

#[derive(Subcommand)]
enum BoardCmd {
    /// Add a card.
    Add {
        title: String,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long, default_value = "todo")]
        status: String,
        #[arg(long)]
        r#type: Option<String>,
    },
    /// List cards.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Show one card in full.
    Show { id: String },
    /// Move a card to done (gate-aware).
    Done {
        id: String,
        /// Acknowledge specific gate criteria as TRUE.
        #[arg(long, num_args = 1..)]
        checked: Vec<String>,
    },
    /// Move a card to doing.
    Doing { id: String },
    /// Move a card back to todo.
    Todo { id: String },
}

#[derive(Subcommand)]
enum WorkerCmd {
    List,
    Start { name: String },
    Stop { name: String },
}

#[derive(Subcommand)]
enum SchedCmd {
    List,
    Run { id: String },
}

struct Client {
    base: String,
    token: Option<String>,
    session: Option<String>,
    http: reqwest::blocking::Client,
}

impl Client {
    fn new(base: String, session: Option<String>) -> Self {
        let token = std::env::var("AMUX_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
            })
            .join("auth-token")
            .pipe(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Client {
            base,
            token,
            session,
            http: reqwest::blocking::Client::builder()
                // Self-signed localhost cert is the product behavior.
                .danger_accept_invalid_certs(true)
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("http client"),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let mut r = self.http.request(method, format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        if let Some(s) = &self.session {
            r = r.header("X-Amux-Session", s);
        }
        r
    }

    fn get(&self, path: &str) -> anyhow::Result<Value> {
        Ok(self.req(reqwest::Method::GET, path).send()?.json()?)
    }

    fn send_json(&self, method: reqwest::Method, path: &str, body: Value) -> anyhow::Result<(u16, Value)> {
        let resp = self.req(method, path).json(&body).send()?;
        let status = resp.status().as_u16();
        let v = resp.json().unwrap_or(Value::Null);
        Ok((status, v))
    }
}

// Small pipe helper so token loading reads top-to-bottom.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn main() {
    let cli = Cli::parse();
    let client = Client::new(cli.url.clone(), cli.session.clone());
    let result = run(&cli.cmd, &client);
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn run(cmd: &Cmd, c: &Client) -> anyhow::Result<i32> {
    match cmd {
        Cmd::Health => {
            let v = c.get("/health")?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(0)
        }
        Cmd::Board { cmd } => board(cmd, c),
        Cmd::Workers { cmd } => workers(cmd, c),
        Cmd::Schedules { cmd } => schedules(cmd, c),
        Cmd::Send { worker, text } => {
            let body_text = match text {
                Some(t) => t.clone(),
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };
            let (status, v) = c.send_json(
                reqwest::Method::POST,
                "/api/messages",
                json!({"target": {"worker_name": worker}, "body": body_text}),
            )?;
            if (200..300).contains(&status) {
                println!("sent to {worker}");
                Ok(0)
            } else {
                eprintln!("send failed ({status}): {v}");
                Ok(3)
            }
        }
    }
}

fn board(cmd: &BoardCmd, c: &Client) -> anyhow::Result<i32> {
    match cmd {
        BoardCmd::Add { title, desc, status, r#type } => {
            let mut body = json!({"title": title, "status": status});
            if let Some(d) = desc {
                body["desc"] = json!(d);
            }
            if let Some(t) = r#type {
                body["type"] = json!(t);
            }
            if let Some(s) = &c.session {
                body["session"] = json!(s);
            }
            let (code, v) = c.send_json(reqwest::Method::POST, "/api/board", body)?;
            if code == 201 {
                println!("{} → {}", v["id"].as_str().unwrap_or("?"), status);
                Ok(0)
            } else {
                eprintln!("create failed ({code}): {v}");
                Ok(3)
            }
        }
        BoardCmd::List { status, session } => {
            let mut path = "/api/board?done_limit=100".to_string();
            if let Some(s) = status {
                path.push_str(&format!("&status={s}"));
            }
            if let Some(s) = session {
                path.push_str(&format!("&session={s}"));
            }
            let v = c.get(&path)?;
            for item in v.as_array().unwrap_or(&vec![]) {
                println!(
                    "{:<12} {:<9} {:<18} {}",
                    item["id"].as_str().unwrap_or("?"),
                    item["status"].as_str().unwrap_or("?"),
                    item["session"].as_str().unwrap_or("-"),
                    item["title"].as_str().unwrap_or("")
                );
            }
            Ok(0)
        }
        BoardCmd::Show { id } => {
            let v = c.get(&format!("/api/board/{id}"))?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(0)
        }
        BoardCmd::Done { id, checked } => move_status(c, id, "done", checked),
        BoardCmd::Doing { id } => move_status(c, id, "doing", &[]),
        BoardCmd::Todo { id } => move_status(c, id, "todo", &[]),
    }
}

/// Status move with LOUD gate handling: a 409 prints the criteria and the
/// exact retry command instead of a silent bounce (AMUX-1769/2325 lessons).
fn move_status(c: &Client, id: &str, status: &str, checked: &[String]) -> anyhow::Result<i32> {
    let mut body = json!({"status": status});
    if !checked.is_empty() {
        body["gate_checked"] = json!(checked);
    }
    let (code, v) = c.send_json(reqwest::Method::PATCH, &format!("/api/board/{id}"), body)?;
    if (200..300).contains(&code) {
        println!("{id} → {status}");
        return Ok(0);
    }
    if code == 409 && v["gate"].is_array() {
        eprintln!("{}", serde_json::to_string_pretty(&v)?);
        eprintln!("\nSatisfy these, then acknowledge the ones that are TRUE:");
        let gate: Vec<String> = v["gate"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|g| g.as_str().map(String::from))
            .collect();
        for g in &gate {
            eprintln!("   [ ] {g}");
        }
        let quoted: Vec<String> = gate.iter().map(|g| format!("{g:?}")).collect();
        eprintln!("\n  amux-rs board done {id} --checked {}", quoted.join(" "));
        return Ok(3);
    }
    eprintln!("move failed ({code}): {v}");
    Ok(3)
}

fn workers(cmd: &WorkerCmd, c: &Client) -> anyhow::Result<i32> {
    match cmd {
        WorkerCmd::List => {
            let v = c.get("/api/workers")?;
            for w in v["items"].as_array().unwrap_or(&vec![]) {
                println!(
                    "{:<22} {:<10} {:<8} {}",
                    w["display_name"].as_str().unwrap_or("?"),
                    w["state"]["state"].as_str().unwrap_or("?"),
                    w["provider"].as_str().unwrap_or("?"),
                    w["id"].as_str().unwrap_or("")
                );
            }
            Ok(0)
        }
        WorkerCmd::Start { name } => {
            let (code, v) = c.send_json(
                reqwest::Method::POST,
                &format!("/api/workers/{name}/start"),
                json!({}),
            )?;
            if code == 202 {
                println!("{name} starting");
                Ok(0)
            } else {
                eprintln!("start failed ({code}): {v}");
                Ok(3)
            }
        }
        WorkerCmd::Stop { name } => {
            let (code, v) = c.send_json(
                reqwest::Method::POST,
                &format!("/api/workers/{name}/stop"),
                json!({}),
            )?;
            if (200..300).contains(&code) {
                println!("{name} stopped");
                Ok(0)
            } else {
                eprintln!("stop failed ({code}): {v}");
                Ok(3)
            }
        }
    }
}

fn schedules(cmd: &SchedCmd, c: &Client) -> anyhow::Result<i32> {
    match cmd {
        SchedCmd::List => {
            let v = c.get("/api/schedules")?;
            let items = v.as_array().cloned().unwrap_or_else(|| {
                v["items"].as_array().cloned().unwrap_or_default()
            });
            for s in items {
                println!(
                    "{:<10} {:<3} {:<24} {}",
                    s["id"].as_str().unwrap_or("?"),
                    if s["enabled"].as_i64().unwrap_or(0) == 1 { "on" } else { "off" },
                    s["schedule_expr"].as_str().unwrap_or("?"),
                    s["title"].as_str().unwrap_or("")
                );
            }
            Ok(0)
        }
        SchedCmd::Run { id } => {
            let (code, v) = c.send_json(
                reqwest::Method::POST,
                &format!("/api/schedules/{id}/run"),
                json!({}),
            )?;
            if (200..300).contains(&code) {
                println!("{id} fired");
                Ok(0)
            } else {
                // Shadow-mode 409 is EXPECTED while the Python scheduler owns
                // firing — print the server's own explanation.
                eprintln!("run refused ({code}): {v}");
                Ok(3)
            }
        }
    }
}
