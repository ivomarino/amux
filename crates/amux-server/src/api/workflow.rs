//! `GET /api/sessions/{name}/workflow` — a worker's configured workflow, DERIVED.
//!
//! AMUX-39. The owner asked for a mermaid diagram that populates when you spin a
//! worker up, "so the user can visualize spot check the workflow".
//!
//! # Why this reads state and not the prompt
//!
//! The obvious build is to render the prose the user typed when configuring the
//! worker. That fails ethos rule 7: a picture drawn from the same sentence that
//! configured the worker cannot disagree with it, so it is a check that cannot
//! fail. Worse than useless — it renders a misconfiguration as a tidy diagram and
//! CONFIRMS it, on the surface the user opened specifically to catch that.
//!
//! So every node here is derived from resolved state: `schedules` rows, the
//! worker's env, the board. The diagram is allowed to disagree with what the user
//! believed they set up, and that disagreement is the entire product.
//!
//! # The flags are the feature
//!
//! A diagram that only draws correctly-configured things is decoration. The
//! `flags` array — and the `warn` class on the nodes it names — is what earns the
//! tab. It fires today: measured on the author's machine, 6 session envs and 0
//! containing `CC_MCP`, which is ethos rule 1's flagship failure (mcp.json shipped
//! six servers; the launcher passed them only when `CC_MCP` was set, so 0 of 101
//! sessions ever received them, invisibly, for months). A prose-derived diagram
//! would have drawn the six servers the user described and shown nothing wrong.
//!
//! # No model call
//!
//! Nodes and edges are computed from rows (ethos rule 2). Spending a model call to
//! draw boxes it can derive is the labelling-with-`claude -p` mistake again.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use super::AppState;

/// A schedule row, already narrowed to what the diagram shows.
struct Sched {
    id: String,
    title: String,
    enabled: bool,
    sched_type: String,
    recurrence: String,
    next_run: String,
    watch: bool,
    trigger_on: String,
}

/// Mermaid label text. Quotes and newlines terminate a `["..."]` label early and
/// produce a diagram that fails to parse — which would render as an empty tab,
/// i.e. a silent failure on the surface whose whole job is to show you something.
fn lbl(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| match c {
            '"' => '\'',
            '\n' | '\r' | '[' | ']' | '{' | '}' | '(' | ')' | '<' | '>' | '|' => ' ',
            c => c,
        })
        .collect();
    let t = s.trim();
    if t.chars().count() > 60 {
        let cut: String = t.chars().take(57).collect();
        format!("{cut}...")
    } else {
        t.to_string()
    }
}

fn schedules_for(conn: &rusqlite::Connection, session: &str) -> rusqlite::Result<Vec<Sched>> {
    let mut st = conn.prepare(
        "SELECT id, COALESCE(title,''), enabled, COALESCE(sched_type,''), \
                COALESCE(recurrence,''), COALESCE(next_run,''), COALESCE(watch,0), \
                COALESCE(trigger_on,'') \
         FROM schedules WHERE session=?1 AND deleted IS NULL ORDER BY id",
    )?;
    let rows = st.query_map([session], |r| {
        Ok(Sched {
            id: r.get::<_, String>(0)?,
            title: r.get::<_, String>(1)?,
            enabled: r.get::<_, i64>(2)? != 0,
            sched_type: r.get::<_, String>(3)?,
            recurrence: r.get::<_, String>(4)?,
            next_run: r.get::<_, String>(5)?,
            watch: r.get::<_, i64>(6)? != 0,
            trigger_on: r.get::<_, String>(7)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

/// Open-set board counts by status. Shares the board's own "open" predicate
/// (not deleted, not archived) rather than re-deriving one — a view that
/// disagrees with the mechanism it describes is worse than no view.
fn board_counts(conn: &rusqlite::Connection, session: &str) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut st = conn.prepare(
        "SELECT status, COUNT(*) FROM issues \
         WHERE session=?1 AND deleted IS NULL AND COALESCE(archived,0)=0 \
         GROUP BY status ORDER BY status",
    )?;
    let rows = st.query_map([session], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.flatten().collect())
}

/// The emitter, split out from the handler so tests drive the REAL one.
/// Returns (mermaid, flags).
fn build_graph(
    name: &str,
    dir: &str,
    mcp: &str,
    groups: &[String],
    scheds: &[Sched],
    counts: &[(String, i64)],
) -> (String, Vec<Value>) {
    let mut flags: Vec<Value> = Vec::new();
    let mut warn_nodes: Vec<String> = Vec::new();
    let mut m = String::from("flowchart TD\n");

    let where_ = if dir.is_empty() { "(no CC_DIR)" } else { dir };
    // SEPARATOR, not `<br/>`. mermaid 11 strips <br/> from labels under
    // securityLevel:'strict' — measured BR-LOST both with and without
    // flowchart.htmlLabels — so the halves ran together: "amux" + "/Users/..."
    // rendered as "amux/Users/...", and "orphan job" + "once" as "orphan jobonce".
    // The fix is NOT to relax securityLevel: these labels carry schedule titles
    // straight out of the DB, and strict sanitisation is what stands between that
    // and the SVG. Guarded by no_html_line_breaks_are_emitted.
    m.push_str(&format!("  W[\"worker: {}  \u{b7}  {}\"]\n", lbl(name), lbl(where_)));

    m.push_str(&format!("  W --> SCH[\"Schedules ({})\"]\n", scheds.len()));
    if scheds.is_empty() {
        m.push_str("  SCH --> SCH_NONE[\"(none - this worker only runs when steered)\"]\n");
        flags.push(json!({
            "level": "info",
            "text": "No schedules bound to this worker. It acts only when a human or another worker steers it."
        }));
    }
    for (i, s) in scheds.iter().enumerate() {
        let nid = format!("S{i}");
        let when = if !s.recurrence.is_empty() {
            s.recurrence.clone()
        } else if !s.trigger_on.is_empty() {
            format!("on: {}", s.trigger_on)
        } else if !s.next_run.is_empty() {
            format!("next {}", s.next_run)
        } else {
            s.sched_type.clone()
        };
        m.push_str(&format!("  SCH --> {nid}[\"{}  \u{b7}  {}\"]\n", lbl(&s.title), lbl(&when)));
        // A disabled schedule is intent without effect — the most common silent
        // misconfiguration, and invisible everywhere else in the UI.
        if !s.enabled {
            warn_nodes.push(nid.clone());
            flags.push(json!({"level":"warn","text":
                format!("Schedule '{}' ({}) is DISABLED - it is configured but will never fire.", s.title, s.id)}));
        } else if s.next_run.is_empty() && s.trigger_on.is_empty() && !s.watch {
            warn_nodes.push(nid.clone());
            flags.push(json!({"level":"warn","text":
                format!("Schedule '{}' ({}) is enabled but has no next run and no trigger - nothing will start it.", s.title, s.id)}));
        }
    }

    m.push_str("  W --> TOOLS[\"Tools / MCP\"]\n");
    if mcp.is_empty() {
        m.push_str("  TOOLS --> T_NONE[\"(no MCP servers reach this worker)\"]\n");
        warn_nodes.push("T_NONE".to_string());
        flags.push(json!({"level":"warn","text":
            "No MCP servers reach this worker (CC_MCP is unset). Servers configured in mcp.json are NOT passed unless CC_MCP is set - this is the failure that left 0 of 101 sessions with MCP for months."}));
    } else {
        for (i, t) in mcp.split(',').map(str::trim).filter(|t| !t.is_empty()).enumerate() {
            m.push_str(&format!("  TOOLS --> T{i}[\"{}\"]\n", lbl(t)));
        }
    }

    let total: i64 = counts.iter().map(|(_, n)| *n).sum();
    m.push_str(&format!("  W --> BOARD[\"Board ({total} open)\"]\n"));
    for (i, (st_, n)) in counts.iter().enumerate() {
        m.push_str(&format!("  BOARD --> B{i}[\"{}: {n}\"]\n", lbl(st_)));
    }

    let scope_txt = if groups.is_empty() {
        "untagged - sees only itself".to_string()
    } else {
        groups.join(", ")
    };
    m.push_str(&format!("  W --> SCOPE[\"Scope: {}\"]\n", lbl(&scope_txt)));

    if !warn_nodes.is_empty() {
        m.push_str("  classDef warn fill:#3b2300,stroke:#ff9800,color:#ffcc80;\n");
        m.push_str(&format!("  class {} warn;\n", warn_nodes.join(",")));
    }
    (m, flags)
}

pub async fn workflow(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> (StatusCode, Json<Value>) {
    let cfg = super::session_verbs::parse_env(&name);
    let dir = cfg.get("CC_DIR").unwrap_or("").to_string();
    let mcp = cfg.get("CC_MCP").unwrap_or("").trim().to_string();
    let groups: Vec<String> = cfg
        .get("CC_TAGS")
        .unwrap_or("")
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    let (scheds, counts) = {
        let conn = match state.store.read() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("store unreadable: {e}") })),
                )
            }
        };
        let s = schedules_for(&conn, &name).unwrap_or_default();
        let c = board_counts(&conn, &name).unwrap_or_default();
        (s, c)
    };

    let (m, flags) = build_graph(&name, &dir, &mcp, &groups, &scheds, &counts);
    let total: i64 = counts.iter().map(|(_, n)| *n).sum();

    (
        StatusCode::OK,
        Json(json!({
            "session": name,
            "mermaid": m,
            "flags": flags,
            "derived_from": {
                "schedules": scheds.len(),
                "board_open": total,
                "mcp": mcp.is_empty() == false,
                "groups": groups,
                "dir": dir,
            }
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::{build_graph, lbl, Sched};

    fn sched(id: &str, title: &str, enabled: bool, recurrence: &str, next_run: &str) -> Sched {
        Sched {
            id: id.into(),
            title: title.into(),
            enabled,
            sched_type: "cron".into(),
            recurrence: recurrence.into(),
            next_run: next_run.into(),
            watch: false,
            trigger_on: String::new(),
        }
    }

    #[test]
    fn lbl_neutralises_mermaid_breakers() {
        // A quote or bracket in a card/schedule title would terminate the label
        // early and break the whole diagram — an empty tab is a silent failure on
        // the one surface meant to show you something.
        assert_eq!(lbl("say \"hi\""), "say 'hi'");
        assert_eq!(lbl("a[b]c"), "a b c");
        assert_eq!(lbl("one\ntwo"), "one two");
    }

    /// mermaid strips `<br/>` under securityLevel:'strict', so a label built with
    /// it silently concatenates its two halves. Guard the whole emitter, not just
    /// the two call sites, so a future node cannot reintroduce it.
    #[test]
    fn no_html_line_breaks_are_emitted() {
        let (m, _) = build_graph(
            "lane", "/w", "chrome", &[],
            &[sched("S1", "nightly", true, "0 9 * * *", "2026-01-01")],
            &[("todo".into(), 1)],
        );
        assert!(!m.contains("<br"), "mermaid drops <br/> under strict; got:\n{m}");
        assert!(m.contains('\u{b7}'), "expected the separator instead; got:\n{m}");
    }

    /// The half that matters: a flagger that always fires is as broken as one that
    /// never does. A correctly-configured worker must emit NOTHING — no flags, and
    /// no `classDef warn`, so the diagram carries no orange either.
    #[test]
    fn a_correctly_configured_worker_is_silent() {
        let (m, flags) = build_graph(
            "good", "/w", "chrome,linear", &["gtm".to_string()],
            &[sched("S1", "healthy", true, "0 * * * *", "2026-01-01T01:00:00")],
            &[("todo".into(), 2)],
        );
        assert!(flags.is_empty(), "expected no flags, got {flags:?}");
        assert!(!m.contains("classDef warn"), "no warn styling expected:\n{m}");
    }

    /// Rebuilt from the two shapes that motivated the feature.
    #[test]
    fn a_disabled_or_unstartable_schedule_is_flagged() {
        let (m, flags) = build_graph(
            "bad", "/w", "chrome", &[],
            &[
                sched("S1", "nightly digest", false, "0 9 * * *", "2026-01-01"),
                sched("S2", "orphan job", true, "", ""),
            ],
            &[],
        );
        assert_eq!(flags.len(), 2, "both schedules should flag: {flags:?}");
        assert!(flags.iter().all(|f| f["level"] == "warn"));
        assert!(m.contains("class S0,S1 warn;"), "both nodes styled:\n{m}");
    }

    /// The rule-1 detector: no MCP reaching a worker is the flagship failure this
    /// tab exists to surface, and it must not fire when MCP IS wired.
    #[test]
    fn missing_mcp_flags_but_present_mcp_does_not() {
        let (_, none) = build_graph("a", "/w", "", &[], &[], &[]);
        assert!(none.iter().any(|f| f["text"].as_str().unwrap_or("").contains("No MCP servers")));
        let (m, some) = build_graph("a", "/w", "chrome", &[], &[], &[]);
        assert!(!some.iter().any(|f| f["text"].as_str().unwrap_or("").contains("No MCP servers")));
        assert!(m.contains("T0[\"chrome\"]"), "mcp node expected:\n{m}");
    }

    #[test]
    fn lbl_truncates_long_titles() {
        let long = "x".repeat(200);
        let out = lbl(&long);
        assert!(out.chars().count() <= 60, "got {} chars", out.chars().count());
        assert!(out.ends_with("..."));
    }
}
