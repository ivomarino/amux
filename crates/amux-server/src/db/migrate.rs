//! Migration runner (RR-0019).
//!
//! Migrations are numbered SQL files embedded at compile time from
//! `crates/amux-server/migrations/`. Applied in order inside one exclusive
//! transaction each, tracked in `_amux_migrations`. Two special forms:
//!
//! - Plain SQL statements: executed as a batch.
//! - `-- ADDCOL: <table> <column> <decl...>` directive lines: applied as
//!   `ALTER TABLE ... ADD COLUMN` ONLY when the column is absent. SQLite has
//!   no `ADD COLUMN IF NOT EXISTS`, and the baseline schema mirrors a live
//!   Python database whose tables may or may not already carry the column —
//!   the directive makes the migration idempotent against both shapes.
//!
//! The Python server must be able to keep opening the same DB file after
//! these run (Phase 11 rollback requirement), so migrations are ADDITIVE
//! ONLY: no drops, no renames, no type changes.

use rusqlite::Connection;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

// Embedded at compile time so the binary is self-contained (single-artifact
// deploy is one of the four reasons this rewrite exists).
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_baseline",
        sql: include_str!("../../migrations/0001_baseline.sql"),
    },
    Migration {
        version: 2,
        name: "0002_rust_additions",
        sql: include_str!("../../migrations/0002_rust_additions.sql"),
    },
    Migration {
        version: 3,
        name: "0003_workers",
        sql: include_str!("../../migrations/0003_workers.sql"),
    },
    Migration {
        version: 4,
        name: "0004_commands",
        sql: include_str!("../../migrations/0004_commands.sql"),
    },
    Migration {
        version: 5,
        name: "0005_memories_snapshots",
        sql: include_str!("../../migrations/0005_memories_snapshots.sql"),
    },
    Migration {
        version: 6,
        name: "0006_turns_messages",
        sql: include_str!("../../migrations/0006_turns_messages.sql"),
    },
    Migration {
        version: 7,
        name: "0007_criteria",
        sql: include_str!("../../migrations/0007_criteria.sql"),
    },
    Migration {
        version: 8,
        name: "0008_event_payload",
        sql: include_str!("../../migrations/0008_event_payload.sql"),
    },
    Migration {
        version: 9,
        name: "0009_media_jobs",
        sql: include_str!("../../migrations/0009_media_jobs.sql"),
    },
    Migration {
        version: 10,
        name: "0010_request_log",
        sql: include_str!("../../migrations/0010_request_log.sql"),
    },
    Migration {
        version: 11,
        name: "0011_conversations",
        sql: include_str!("../../migrations/0011_conversations.sql"),
    },
    Migration {
        version: 12,
        name: "0012_invariants",
        sql: include_str!("../../migrations/0012_invariants.sql"),
    },
    Migration {
        version: 13,
        name: "0013_search",
        sql: include_str!("../../migrations/0013_search.sql"),
    },
    Migration {
        version: 14,
        name: "0014_message_delivery_meta",
        sql: include_str!("../../migrations/0014_message_delivery_meta.sql"),
    },
    Migration {
        version: 15,
        name: "0015_schedule_run_delivery",
        sql: include_str!("../../migrations/0015_schedule_run_delivery.sql"),
    },
    Migration {
        version: 16,
        name: "0016_submit_verdict",
        sql: include_str!("../../migrations/0016_submit_verdict.sql"),
    },
];

/// Migrations embedded in THIS binary that the DB has not recorded yet.
///
/// Deliberately mirrors `apply_all`'s own "already applied?" test, including
/// the `unwrap_or(false)` — if `_amux_migrations` does not exist yet the query
/// fails and everything is pending, which is the truth for a fresh DB. A
/// separate predicate here would be a second spelling that drifts.
pub fn pending(conn: &Connection) -> Vec<&'static str> {
    MIGRATIONS
        .iter()
        .filter(|m| {
            !conn
                .query_row(
                    "SELECT 1 FROM _amux_migrations WHERE version = ?1",
                    [m.version],
                    |_| Ok(true),
                )
                .unwrap_or(false)
        })
        .map(|m| m.name)
        .collect()
}

/// True when this binary is a `cargo build` artifact run straight from a
/// source checkout, rather than an installed one.
///
/// The installed server lives at `~/.local/bin/amux-server-rs`; the auto-build
/// compiles into `~/.amux/rust-build-target/release/` and then INSTALLS, so a
/// running server is never the target artifact. Note that build dir contains
/// the substring `target` — matching on `/target/release/` with the leading
/// slash is what keeps `rust-build-target/release/` from tripping this, and
/// there is a test pinning exactly that.
fn is_cargo_target_build(exe: &std::path::Path) -> bool {
    let s = exe.to_string_lossy();
    s.contains("/target/debug/") || s.contains("/target/release/")
}

/// True when `db_path` is the fleet's real database, `$HOME/.amux/amux.db`.
///
/// Compared after `canonicalize` where possible so `AMUX_HOME=~/.amux` (an
/// explicit spelling of the same file) is still recognised as live; a plain
/// string compare would miss it and the guard would not fire.
fn is_live_db(db_path: &std::path::Path, home: Option<&std::path::Path>) -> bool {
    let Some(home) = home else { return false };
    let live = home.join(".amux").join("amux.db");
    let norm = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    norm(db_path) == norm(&live)
}

/// AMUX-2652: a peer's ordinary `cargo run` must not migrate the LIVE database.
///
/// Migrations are `include_str!`-embedded at compile time, so a working-tree
/// build carries whatever migration files that tree has — including a
/// half-written one nobody has committed. The default DB path is the live file,
/// so `cargo run -p amux-server` in this checkout applied 0013_search.sql to
/// ~/.amux/amux.db at 22:18:23, 101 seconds after its author created it and
/// before they had made their read-only copy. It applied cleanly that time.
///
/// The point is not that it broke something; it is that no agent can honour
/// "never touch the live DB" while a PEER's build does it on their behalf. The
/// person who runs the command and the person whose migration runs are
/// different people, so care cannot prevent it — only a check can.
///
/// Scoped as narrowly as the hazard: it fires only for an UNAPPLIED migration,
/// from a cargo target build, against the live file. Anything else — the
/// installed server, a temp `AMUX_HOME`, a container (whose binary is not a
/// target artifact), a dev run with pending migrations against a scratch DB —
/// is untouched. Refusing too eagerly would be worse than the hazard: a server
/// that will not start is a live outage, and this must never be that.
fn guard_live_db(db_path: &std::path::Path, pending: &[&str]) -> anyhow::Result<()> {
    if pending.is_empty() || truthy_env("AMUX_ALLOW_LIVE_DB") {
        return Ok(());
    }
    let Ok(exe) = std::env::current_exe() else {
        return Ok(()); // cannot tell what we are: do not block a real server
    };
    if !is_cargo_target_build(&exe) {
        return Ok(());
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if !is_live_db(db_path, home.as_deref()) {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to apply {n} unapplied migration(s) to the LIVE database.\n\
         \n\
         Migration(s): {names}\n\
         Database    : {db}\n\
         Binary      : {exe} (a cargo target build, not the installed server)\n\
         \n\
         Migrations are embedded at COMPILE time, so this build carries whatever\n\
         is in its working tree — possibly a peer's uncommitted, half-written\n\
         migration. Applying it here would change the schema of the database the\n\
         whole fleet is using, on their behalf and without their knowledge.\n\
         \n\
         Point it somewhere else instead:\n\
           AMUX_HOME=$(mktemp -d) cargo run -p amux-server\n\
           AMUX_DB=/tmp/scratch.db cargo run -p amux-server\n\
         \n\
         Or, if you genuinely mean to migrate the live DB from a working-tree\n\
         build, say so explicitly: AMUX_ALLOW_LIVE_DB=1",
        n = pending.len(),
        names = pending.join(", "),
        db = db_path.display(),
        exe = exe.display(),
    )
}

fn truthy_env(key: &str) -> bool {
    std::env::var(key)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !v.is_empty() && v != "0" && v != "false" && v != "no"
        })
        .unwrap_or(false)
}

/// Guard + apply. `Store::open` calls this; the bare [`apply_all`] stays for
/// tests, which run against temp and in-memory databases.
pub fn apply_all_guarded(conn: &mut Connection, db_path: &std::path::Path) -> anyhow::Result<()> {
    guard_live_db(db_path, &pending(conn))?;
    apply_all(conn)
}

pub fn apply_all(conn: &mut Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _amux_migrations (
            version    INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;
    for m in MIGRATIONS {
        let already: bool = conn
            .query_row(
                "SELECT 1 FROM _amux_migrations WHERE version = ?1",
                [m.version],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if already {
            continue;
        }
        let tx = conn.transaction()?;
        apply_one(&tx, m.sql)?;
        tx.execute(
            "INSERT INTO _amux_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![m.version, m.name, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
    }
    Ok(())
}

fn apply_one(conn: &Connection, sql: &str) -> anyhow::Result<()> {
    // Split ADDCOL directives from plain SQL. Directives are full-line
    // comments so the file stays valid SQL for external tools.
    let mut plain = String::new();
    let mut addcols: Vec<(String, String, String)> = Vec::new();
    for line in sql.lines() {
        if let Some(rest) = line.trim().strip_prefix("-- ADDCOL:") {
            let mut parts = rest.trim().splitn(3, ' ');
            let (Some(table), Some(column), Some(decl)) =
                (parts.next(), parts.next(), parts.next())
            else {
                anyhow::bail!("malformed ADDCOL directive: {line:?}");
            };
            addcols.push((table.to_string(), column.to_string(), decl.to_string()));
        } else {
            plain.push_str(line);
            plain.push('\n');
        }
    }
    if !plain.trim().is_empty() {
        conn.execute_batch(&plain)?;
    }
    for (table, column, decl) in addcols {
        if !column_exists(conn, &table, &column)? {
            // Identifiers cannot be bound as parameters; they come from our
            // own embedded migration files, not user input.
            conn.execute_batch(&format!(
                "ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {decl};"
            ))?;
        }
    }
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_to_fresh_db_and_are_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_all(&mut conn).unwrap();
        // Applying again is a no-op, not an error.
        apply_all(&mut conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM _amux_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n as usize, super::MIGRATIONS.len());
        // The revision row exists and starts at 0.
        let rev: u64 = conn
            .query_row("SELECT rev FROM _amux_rev WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rev, 0);
    }

    #[test]
    fn addcol_directive_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (a INTEGER);").unwrap();
        let sql = "-- ADDCOL: t b INTEGER NOT NULL DEFAULT 0\n";
        apply_one(&conn, sql).unwrap();
        apply_one(&conn, sql).unwrap(); // second run: column exists, skipped
        assert!(column_exists(&conn, "t", "b").unwrap());
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    // The near-miss that would silently disable the guard for the INSTALLED
    // server's build dir: ~/.amux/rust-build-target/release/ contains the
    // substring "target", so a naive contains("target/release") matches it.
    // The leading slash is the whole defence, and this pins it.
    #[test]
    fn the_auto_build_target_dir_is_not_mistaken_for_a_cargo_run() {
        assert!(is_cargo_target_build(Path::new(
            "/Users/x/Dev/amux/target/debug/amux-server"
        )));
        assert!(is_cargo_target_build(Path::new(
            "/Users/x/Dev/amux/target/release/amux-server"
        )));
        assert!(
            !is_cargo_target_build(Path::new(
                "/Users/x/.amux/rust-build-target/release/amux-server"
            )),
            "the auto-build's own target dir must not read as a cargo run — \
             it INSTALLS, and blocking it would stop the real server booting"
        );
        assert!(!is_cargo_target_build(Path::new(
            "/Users/x/.local/bin/amux-server-rs"
        )));
        // A container: binary on PATH, no target dir anywhere.
        assert!(!is_cargo_target_build(Path::new("/usr/local/bin/amux-server")));
    }

    #[test]
    fn only_the_real_home_db_counts_as_live() {
        let home = PathBuf::from("/Users/x");
        assert!(is_live_db(Path::new("/Users/x/.amux/amux.db"), Some(&home)));
        assert!(!is_live_db(Path::new("/tmp/scratch/amux.db"), Some(&home)));
        assert!(!is_live_db(Path::new("/Users/x/.amux/other.db"), Some(&home)));
        // No HOME at all: cannot prove it is live, so do not block.
        assert!(!is_live_db(Path::new("/Users/x/.amux/amux.db"), None));
    }

    // The guard must be silent in every legitimate case, or it becomes an
    // outage. A refusal that fires on the installed server is strictly worse
    // than the hazard it prevents.
    #[test]
    fn nothing_pending_never_refuses_even_on_the_live_db() {
        assert!(guard_live_db(Path::new("/Users/x/.amux/amux.db"), &[]).is_ok());
    }

    #[test]
    fn a_scratch_db_is_never_refused_however_it_was_built() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("amux.db");
        assert!(guard_live_db(&db, &["0099_experiment"]).is_ok());
    }

    #[test]
    fn truthy_env_treats_the_usual_falsey_spellings_as_off() {
        for (v, want) in [
            ("1", true),
            ("true", true),
            ("YES", true),
            ("0", false),
            ("false", false),
            ("no", false),
            ("", false),
            ("  ", false),
        ] {
            std::env::set_var("AMUX_TEST_TRUTHY", v);
            assert_eq!(truthy_env("AMUX_TEST_TRUTHY"), want, "value {v:?}");
        }
        std::env::remove_var("AMUX_TEST_TRUTHY");
        assert!(!truthy_env("AMUX_TEST_TRUTHY"));
    }

    // THE POSITIVE CASE. Everything above proves the guard stays QUIET; a guard
    // that only ever returns Ok would pass all of them. This is the one that
    // proves it can fire.
    //
    // Safe to run against the real live path because guard_live_db only
    // INSPECTS paths — it never opens, reads, or migrates the database. And the
    // test binary is itself a cargo target artifact (target/debug/deps/...),
    // which is exactly the build shape the hazard describes, so this exercises
    // the real predicate rather than a stubbed one.
    #[test]
    fn it_actually_refuses_a_pending_migration_against_the_live_db() {
        let exe = std::env::current_exe().expect("current_exe");
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            panic!("HOME unset — this test cannot express its precondition, \
                    which is a broken test, not a passing one");
        };
        assert!(
            is_cargo_target_build(&exe),
            "precondition: the test binary should live under a cargo target dir, got {}",
            exe.display()
        );

        let live = home.join(".amux").join("amux.db");
        std::env::remove_var("AMUX_ALLOW_LIVE_DB");
        let err = guard_live_db(&live, &["0099_pretend_uncommitted"])
            .expect_err("the guard MUST refuse a pending migration against the live DB");
        let msg = err.to_string();
        assert!(msg.contains("LIVE database"), "{msg}");
        assert!(msg.contains("0099_pretend_uncommitted"), "{msg}");
        // The refusal has to publish its own escape, or the next agent
        // hand-rolls something worse to get past it (the AMUX-2325 lesson).
        assert!(msg.contains("AMUX_ALLOW_LIVE_DB=1"), "{msg}");
        assert!(msg.contains("AMUX_HOME"), "{msg}");

        // The sanctioned escape must actually work, or the message above is a
        // lie and the guard is unwalkable (the AMUX-2325 shape: a constraint
        // whose documented exit does not exist gets routed around by something
        // worse). Same test, so nothing can race on this env var.
        std::env::set_var("AMUX_ALLOW_LIVE_DB", "1");
        let permitted = guard_live_db(&live, &["0099_pretend_uncommitted"]);
        std::env::remove_var("AMUX_ALLOW_LIVE_DB");
        assert!(
            permitted.is_ok(),
            "AMUX_ALLOW_LIVE_DB=1 must permit it: {permitted:?}"
        );
    }

    // NOTE: the refuse and permit cases are ONE test on purpose. As two, they
    // raced on AMUX_ALLOW_LIVE_DB — process-global env, threaded harness — and
    // the escape case failed roughly at random. That is precisely the flake
    // class AMUX-2675 was about; merging removes it by construction instead of
    // adding another lock that the next test must remember to take.
}
