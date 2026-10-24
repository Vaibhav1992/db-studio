//! SQL Server access over TDS (tiberius). One TCP connection per session (query tab / explorer db).
use crate::{sqlguard, store::Conn};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use serde::Serialize;
use std::{collections::HashMap, sync::{Mutex, OnceLock}, time::{Duration, Instant}};
use tauri::{AppHandle, Emitter, Manager};
use tiberius::{AuthMethod, Client, ColumnData, Config, FromSql, QueryItem};
use tokio::{net::TcpStream, task::AbortHandle};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub type Tds = Client<Compat<TcpStream>>;
const HARD_CAP: usize = 200_000; // per result set, protects UI memory when the limit is "all"
const ROWCOUNT_COL: &str = "__dbstudio_rowcount";
const OPEN_TRAN: u32 = 50_871; // our THROW code for "tab has an open transaction"

type Key = (Conn, Option<String>); // full record: an edited server/auth must not reuse the old socket

/// The database a connection really opens, so a pinned connection's tab and its tree share one key.
fn key(c: &Conn, database: Option<&str>) -> Key {
    (c.clone(), database.or(c.database.as_deref()).filter(|d| !d.is_empty()).map(String::from))
}

struct Session {
    key: Key,
    client: Tds,
}

#[derive(Default)]
pub struct Db {
    // A session's client is taken out while a query runs and put back after; absent = busy or new.
    sessions: Mutex<HashMap<String, Session>>,
    running: Mutex<HashMap<String, AbortHandle>>,
    tokens: tokio::sync::Mutex<HashMap<String, (String, i64)>>, // tenant -> (token, expires_on unix)
    spares: Mutex<Vec<(Key, Option<Tds>)>>, // one pre-opened connection per key; None = still opening
    pub app: OnceLock<AppHandle>, // unset in tests: no events, no pre-opened connections
}

/// Where a call's time goes: (step, ms) pairs. Live progress goes to the UI as the `dbs-phase` event.
pub struct Trace {
    app: Option<AppHandle>,
    session: String,
    last: Instant,
    steps: Vec<(String, u64)>,
}

#[derive(Serialize, Clone)]
struct Phase<'a> {
    session: &'a str,
    now: Option<&'a str>, // None = finished
    steps: &'a [(String, u64)],
}

impl Trace {
    pub fn new(db: &Db, session: &str) -> Self {
        Trace { app: db.app.get().cloned(), session: session.into(), last: Instant::now(), steps: vec![] }
    }
    pub fn quiet() -> Self {
        Trace { app: None, session: String::new(), last: Instant::now(), steps: vec![] }
    }
    /// Start the phase `now` (None = finished) and tell the UI.
    fn go(&mut self, now: Option<&str>) {
        self.last = Instant::now();
        if let Some(a) = &self.app {
            let _ = a.emit("dbs-phase", Phase { session: &self.session, now, steps: &self.steps });
        }
    }
    /// End the current phase as step `name`; repeated steps (one per GO batch) add up.
    fn done(&mut self, name: &str) {
        let ms = self.last.elapsed().as_millis() as u64;
        match self.steps.iter_mut().find(|s| s.0 == name) {
            Some(s) => s.1 += ms,
            None => self.steps.push((name.into(), ms)),
        }
    }
}

#[derive(Serialize, Default, Debug)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    truncated: bool,
}

#[derive(Serialize, Default, Debug)]
pub struct QueryResult {
    pub results: Vec<ResultSet>,
    pub messages: Vec<String>,
    elapsed_ms: u64,
    steps: Vec<(String, u64)>,
}

pub fn fmt_err(e: tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(t) => format!(
            "Msg {}, Level {}, State {}, Line {}\n{}",
            t.code(), t.class(), t.state(), t.line(), t.message()
        ),
        e => e.to_string(),
    }
}

impl Db {
    /// Entra ID token for Azure SQL from the user's `az login` session, cached until 5 min before expiry.
    /// Returns (token, came from cache).
    pub async fn azure_token(&self, tenant: Option<&str>) -> Result<(String, bool), String> {
        let key = tenant.unwrap_or("").to_string();
        let mut cache = self.tokens.lock().await;
        if let Some((tok, exp)) = cache.get(&key) {
            if *exp - 300 > chrono::Utc::now().timestamp() {
                return Ok((tok.clone(), true));
            }
        }
        let mut cmd = tokio::process::Command::new(az_bin());
        cmd.args(["account", "get-access-token", "--resource", "https://database.windows.net/", "-o", "json"]);
        if let Some(t) = tenant {
            cmd.args(["--tenant", t]);
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
            .await
            .map_err(|_| "Azure CLI token request timed out (60 s).".to_string())?
            .map_err(|e| format!("Azure CLI (`az`) not found ({e}). Install it with `brew install azure-cli`, then run `az login`."))?;
        if !out.status.success() {
            return Err(format!("Azure CLI token failed. Run `az login` in a terminal.\n{}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
        let tok = v["accessToken"].as_str().ok_or("az: no accessToken")?.to_string();
        let exp = v["expires_on"].as_i64().unwrap_or_else(|| chrono::Utc::now().timestamp() + 1800);
        cache.insert(key, (tok.clone(), exp));
        Ok((tok, false))
    }

    pub async fn connect(&self, c: &Conn, password: Option<String>, database: Option<&str>, tr: &mut Trace) -> Result<Tds, String> {
        let mut cfg = Config::new();
        cfg.host(&c.server);
        cfg.port(c.port);
        cfg.application_name("DB Studio");
        if let Some(db) = database.or(c.database.as_deref()).filter(|d| !d.is_empty()) {
            cfg.database(db);
        }
        if c.trust_cert {
            cfg.trust_cert();
        }
        cfg.readonly(c.read_only);
        cfg.authentication(match c.auth.as_str() {
            "azure_cli" => {
                tr.go(Some("Getting Azure token"));
                let (tok, cached) = self.azure_token(c.tenant.as_deref().filter(|t| !t.is_empty())).await?;
                tr.done(if cached { "Azure token (cached)" } else { "Azure token" });
                AuthMethod::aad_token(tok)
            }
            _ => {
                let pw = match password {
                    Some(p) => Some(p),
                    None => crate::store::password_for(c)?,
                };
                let pw = pw.ok_or("No saved password for this connection. Right-click it, select Edit, and enter the password.")?;
                AuthMethod::sql_server(c.user.clone().unwrap_or_default(), pw)
            }
        });
        // Azure SQL may redirect to the node that holds the database; follow it once.
        for hop in ["", " (redirected)"] {
            tr.go(Some("Connecting (TCP)"));
            let tcp = tokio::time::timeout(Duration::from_secs(15), TcpStream::connect(cfg.get_addr()))
                .await
                .map_err(|_| format!("Timed out connecting to {}", cfg.get_addr()))?
                .map_err(|e| format!("{}: {e}", cfg.get_addr()))?;
            tr.done(&format!("TCP{hop}"));
            tcp.set_nodelay(true).ok();
            // Keeps idle sessions and pre-opened connections alive through VPN/NAT and the Azure gateway idle timeout.
            let ka = socket2::TcpKeepalive::new().with_time(Duration::from_secs(30)).with_interval(Duration::from_secs(10));
            socket2::SockRef::from(&tcp).set_tcp_keepalive(&ka).ok();
            tr.go(Some("Logging in (TLS + auth)"));
            match Client::connect(cfg.clone(), tcp.compat_write()).await {
                Ok(client) => {
                    tr.done(&format!("Login{hop}"));
                    return Ok(client);
                }
                Err(tiberius::error::Error::Routing { host, port }) => {
                    tr.done("Login, then redirect");
                    cfg.host(&host);
                    cfg.port(port);
                }
                Err(e) => return Err(fmt_err(e)),
            }
        }
        Err("Too many redirects".into())
    }

    async fn take(&self, session: &str, c: &Conn, database: Option<&str>, tr: &mut Trace) -> Result<Tds, String> {
        let key = key(c, database);
        let existing = self.sessions.lock().unwrap().remove(session);
        if let Some(s) = existing.filter(|s| s.key == key) {
            tr.done("Connection reused");
            return Ok(s.client);
        }
        let spare = {
            let mut sp = self.spares.lock().unwrap();
            sp.iter().position(|(k, s)| *k == key && s.is_some()).and_then(|i| sp.swap_remove(i).1)
        };
        let client = match spare {
            Some(cl) => {
                tr.done("Pre-opened connection");
                cl
            }
            None => self.connect(c, None, database, tr).await?,
        };
        self.refill(key);
        Ok(client)
    }

    /// Open one spare connection per key in the background, so the next new tab or
    /// "Select Top" on that database skips the TCP + TLS + login round trips.
    fn refill(&self, key: Key) {
        let Some(app) = self.app.get().cloned() else { return };
        {
            let mut sp = self.spares.lock().unwrap();
            if sp.iter().any(|(k, _)| *k == key) {
                return;
            }
            sp.retain(|(k, _)| k.0.id != key.0.id || k.0 == key.0); // spares of an edited connection
            sp.push((key.clone(), None));
        }
        tauri::async_runtime::spawn(async move {
            let db = app.state::<Db>();
            let r = db.connect(&key.0, None, key.1.as_deref(), &mut Trace::quiet()).await;
            let mut sp = db.spares.lock().unwrap();
            let slot = sp.iter().position(|(k, s)| *k == key && s.is_none());
            match (r, slot) {
                (Ok(cl), Some(i)) => sp[i].1 = Some(cl),
                (Err(_), Some(i)) => drop(sp.swap_remove(i)),
                _ => {} // slot removed by close(): drop the new connection
            }
        });
    }

    fn put(&self, session: &str, c: &Conn, database: Option<&str>, client: Tds) {
        let key = key(c, database);
        self.sessions.lock().unwrap().insert(session.into(), Session { key, client });
    }

    pub fn close(&self, session: &str) {
        self.sessions.lock().unwrap().remove(session);
        // Closing a database's explorer session (before drop database) also closes its spare connections.
        if let Some((id, db)) = session.strip_prefix("explorer:").and_then(|s| s.split_once(':')) {
            self.spares.lock().unwrap().retain(|(k, _)| !(k.0.id == id && k.1.as_deref() == Some(db)));
        }
    }

    pub fn cancel(&self, session: &str) {
        if let Some(h) = self.running.lock().unwrap().remove(session) {
            h.abort(); // drops the client mid-stream; the server stops when the socket closes
        }
    }

    /// Run a small metadata query on a per-(conn, db) explorer session.
    pub async fn meta(&self, c: &Conn, database: Option<&str>, sql: &str, params: &[&str]) -> Result<Vec<Vec<Option<String>>>, String> {
        let session = format!("explorer:{}:{}", c.id, database.unwrap_or(""));
        let mut tr = Trace::new(self, &session);
        let mut client = self.take(&session, c, database, &mut tr).await?;
        tr.go(Some("Loading"));
        let p: Vec<&dyn tiberius::ToSql> = params.iter().map(|s| s as &dyn tiberius::ToSql).collect();
        let rows = async { client.query(sql, &p).await?.into_first_result().await }.await.map_err(fmt_err)?;
        tr.done("Query");
        tr.go(None);
        self.put(&session, c, database, client);
        Ok(rows.into_iter().map(|r| r.into_iter().map(|cd| fmt_value(&cd)).collect()).collect())
    }

    /// `max_rows` caps every result set (0 = HARD_CAP).
    pub async fn run(&self, c: &Conn, database: Option<&str>, session: &str, sql: &str, read_only: bool, max_rows: usize) -> Result<QueryResult, String> {
        let max = if max_rows == 0 { HARD_CAP } else { max_rows.min(HARD_CAP) };
        let batches = sqlguard::split_batches(sql);
        if batches.is_empty() {
            return Err("Nothing to run.".into());
        }
        if read_only {
            for b in &batches {
                if let Some(why) = sqlguard::write_reason(b) {
                    return Err(format!("Blocked: read-only mode allows only SELECT-style queries ({why})."));
                }
            }
        }
        let mut tr = Trace::new(self, session);
        let mut client = self.take(session, c, database, &mut tr).await?;
        let task = tokio::spawn(async move {
            let r = run_batches(&mut client, &batches, read_only, max, &mut tr).await;
            tr.go(None);
            (client, r, tr.steps)
        });
        self.running.lock().unwrap().insert(session.into(), task.abort_handle());
        let joined = task.await;
        self.running.lock().unwrap().remove(session);
        match joined {
            Err(_) => Err("Query cancelled. The connection was closed; a statement already running on the server may still finish.".into()),
            Ok((client, Ok((mut res, reusable)), steps)) => {
                if reusable {
                    self.put(session, c, database, client);
                }
                res.steps = steps;
                Ok(res)
            }
            Ok((client, Err((e, reusable)), _)) => {
                if reusable {
                    self.put(session, c, database, client);
                }
                Err(e)
            }
        }
    }
}

/// Apps opened from Finder get PATH=/usr/bin:/bin:/usr/sbin:/sbin, which misses Homebrew's az.
fn az_bin() -> std::path::PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(std::path::PathBuf::from))
        .map(|d| d.join("az"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| "az".into())
}

/// Ok/Err carry `reusable`: false when the connection is mid-stream or broken and must be dropped.
/// Session invariant between batches: SET ROWCOUNT 0 and no transaction opened by us.
async fn run_batches(client: &mut Tds, batches: &[String], read_only: bool, max: usize, tr: &mut Trace) -> Result<(QueryResult, bool), (String, bool)> {
    let start = Instant::now();
    let mut out = QueryResult::default();
    let n = batches.len();
    for (i, b) in batches.iter().enumerate() {
        // Prefixes stay on the user's first line so server error line numbers still match the editor.
        // Read batches get a server-side row limit (SET ROWCOUNT) so SELECT * on a big table stays cheap;
        // never on writes, where ROWCOUNT would silently limit UPDATE/DELETE.
        let reads = sqlguard::write_reason(b).is_none();
        let (text, cleanup) = if read_only {
            // Second layer behind the keyword guard: anything that slips through is rolled back.
            // Refuse if the tab holds the user's own open transaction: our ROLLBACK would discard it.
            let pre = format!("IF @@TRANCOUNT > 0 THROW {OPEN_TRAN}, 'This tab has an open transaction. Commit or roll back before a read-only run.', 1; BEGIN TRAN; SET ROWCOUNT {}; ", max + 1);
            (format!("{pre}{b}\n;SET ROWCOUNT 0; IF @@TRANCOUNT > 0 ROLLBACK"), "SET ROWCOUNT 0; IF @@TRANCOUNT > 0 ROLLBACK")
        } else if reads {
            (format!("SET ROWCOUNT {}; {b}\n;SET ROWCOUNT 0", max + 1), "SET ROWCOUNT 0")
        } else if sqlguard::wants_rowcount(b) {
            (format!("{b}\n;SELECT @@ROWCOUNT AS [{ROWCOUNT_COL}]"), "")
        } else {
            (b.clone(), "")
        };
        let r = run_one(client, &text, &mut out, max, read_only || reads, tr).await;
        // The in-batch cleanup did not run after an error. A mid-stream or broken connection is dropped
        // instead; the server then resets it on its own.
        if let Err(tiberius::error::Error::Server(e)) = &r {
            if !cleanup.is_empty() && e.code() != OPEN_TRAN {
                if let Ok(s) = client.simple_query(cleanup).await {
                    let _ = s.into_results().await;
                }
            }
        }
        match r {
            Ok(true) => {}
            Ok(false) => {
                out.messages.push(format!("Stopped after {max} rows; remaining results skipped and connection reset."));
                out.elapsed_ms = start.elapsed().as_millis() as u64;
                return Ok((out, false));
            }
            Err(e) => {
                let reusable = matches!(e, tiberius::error::Error::Server(_));
                let prefix = if n > 1 { format!("Batch {} of {n} failed ({i} ran before it):\n", i + 1) } else { String::new() };
                let warn = if reads { "" } else { "\nStatements before the error may have run and committed." };
                return Err((format!("{prefix}{}{warn}", fmt_err(e)), reusable));
            }
        }
    }
    if out.results.is_empty() && out.messages.is_empty() {
        out.messages.push("Commands completed successfully.".into());
    }
    out.elapsed_ms = start.elapsed().as_millis() as u64;
    Ok((out, true))
}

/// Ok(false) = row cap hit and the stream was abandoned. `server_capped`: SET ROWCOUNT max+1 is active,
/// so at most one extra row per result set arrives and we can keep reading.
async fn run_one(client: &mut Tds, sql: &str, out: &mut QueryResult, max: usize, server_capped: bool, tr: &mut Trace) -> tiberius::Result<bool> {
    tr.go(Some("Waiting for server"));
    let mut stream = client.simple_query(sql).await?.into_stream();
    let mut first = true; // "Server" = until the first response; "Rows" = the rest of the stream
    while let Some(item) = stream.try_next().await? {
        if first {
            first = false;
            tr.done("Server");
            tr.go(Some("Receiving rows"));
        }
        match item {
            QueryItem::Metadata(m) => out.results.push(ResultSet {
                columns: m.columns().iter().map(|c| c.name().to_string()).collect(),
                ..Default::default()
            }),
            QueryItem::Row(row) => {
                let rs = out.results.last_mut().expect("row before metadata");
                if rs.rows.len() >= max {
                    rs.truncated = true;
                    if server_capped {
                        continue;
                    }
                    tr.done("Rows");
                    return Ok(false);
                }
                rs.rows.push(row.into_iter().map(|cd| fmt_value(&cd)).collect());
            }
        }
    }
    tr.done(if first { "Server" } else { "Rows" });
    // Turn our appended `SELECT @@ROWCOUNT` into a message.
    if out.results.last().is_some_and(|r| r.columns == [ROWCOUNT_COL]) {
        let rs = out.results.pop().unwrap();
        let n = rs.rows.first().and_then(|r| r[0].clone()).unwrap_or_default();
        out.messages.push(format!("({n} rows affected by the last statement)"));
    }
    Ok(true)
}

fn fmt_value(cd: &ColumnData<'static>) -> Option<String> {
    fn t<T: for<'a> FromSql<'a>>(cd: &ColumnData<'static>) -> Option<T> {
        T::from_sql(cd).ok().flatten()
    }
    const DT: &str = "%Y-%m-%d %H:%M:%S%.3f";
    match cd {
        ColumnData::U8(v) => v.map(|x| x.to_string()),
        ColumnData::I16(v) => v.map(|x| x.to_string()),
        ColumnData::I32(v) => v.map(|x| x.to_string()),
        ColumnData::I64(v) => v.map(|x| x.to_string()),
        ColumnData::F32(v) => v.map(|x| x.to_string()),
        ColumnData::F64(v) => v.map(|x| x.to_string()),
        ColumnData::Bit(v) => v.map(|x| if x { "1" } else { "0" }.into()),
        ColumnData::String(v) => v.as_ref().map(|s| s.to_string()),
        ColumnData::Guid(v) => v.map(|g| g.to_string().to_uppercase()),
        ColumnData::Numeric(v) => v.map(|n| n.to_string()),
        ColumnData::Xml(v) => v.as_ref().map(|x| x.to_string()),
        ColumnData::Binary(v) => v.as_ref().map(|b| {
            let mut s: String = "0x".to_string() + &b.iter().take(1024).map(|x| format!("{x:02X}")).collect::<String>();
            if b.len() > 1024 {
                s.push('…');
            }
            s
        }),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            t::<NaiveDateTime>(cd).map(|d| d.format(DT).to_string())
        }
        ColumnData::Date(_) => t::<NaiveDate>(cd).map(|d| d.to_string()),
        ColumnData::Time(_) => t::<NaiveTime>(cd).map(|d| d.to_string()),
        ColumnData::DateTimeOffset(_) => t::<DateTime<FixedOffset>>(cd).map(|d| d.format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string()),
    }
}

#[cfg(test)]
mod live {
    //! Localhost only. Run: DBS_SA_PW=... cargo test live -- --ignored --nocapture
    use super::*;

    fn conn(server: &str, auth: &str, db: Option<&str>) -> Conn {
        Conn {
            id: "t".into(), name: "t".into(), server: server.into(), port: 1433, database: db.map(String::from),
            auth: auth.into(), user: Some("sa".into()), tenant: None, env: "local".into(), read_only: false, trust_cert: true,
        }
    }

    #[tokio::test]
    #[ignore]
    async fn local_sql() {
        let db = Db::default();
        let c = conn("localhost", "sql", Some("tempdb"));
        let pw = std::env::var("DBS_SA_PW").unwrap();
        db.put("s", &c, None, db.connect(&c, Some(pw), None, &mut Trace::quiet()).await.unwrap());
        let r = db.run(&c, None, "s", "CREATE TABLE #t(id int, g uniqueidentifier, d datetime2, m decimal(10,2), b varbinary(4), x nvarchar(10))\nGO\n\
            INSERT #t VALUES (1, NEWID(), '2024-01-02 03:04:05.678', 12.5, 0x0A0B, N'héllo'),(2,NULL,NULL,NULL,NULL,NULL)\nGO\nSELECT * FROM #t; SELECT 1 a, 2 b", false, 1000).await.unwrap();
        println!("{}", serde_json::to_string(&r).unwrap());
        assert_eq!(r.results.len(), 2);
        assert!(r.messages.contains(&"(2 rows affected by the last statement)".to_string()));
        assert_eq!(r.results[0].rows[0][2].as_deref(), Some("2024-01-02 03:04:05.678"));
        assert_eq!(r.results[0].rows[1][1], None);
        // Session kept #t alive across runs; read-only blocks writes and SELECT works.
        assert!(db.run(&c, None, "s", "DELETE FROM #t", true, 1000).await.unwrap_err().starts_with("Blocked"));
        assert_eq!(db.run(&c, None, "s", "SELECT COUNT(*) FROM #t", true, 1000).await.unwrap().results[0].rows[0][0].as_deref(), Some("2"));
        let e = db.run(&c, None, "s", "SELECT * FROM nope", false, 1000).await.unwrap_err();
        assert!(e.contains("Msg 208"), "{e}");
        // Cancel aborts a long query.
        let t0 = Instant::now();
        let (r, _) = tokio::join!(db.run(&c, None, "s", "WAITFOR DELAY '00:00:10'", false, 1000), async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            db.cancel("s")
        });
        assert!(r.unwrap_err().starts_with("Query cancelled."));
        assert!(t0.elapsed().as_secs() < 2);
        // Row cap.
        db.put("s2", &c, None, db.connect(&c, Some(std::env::var("DBS_SA_PW").unwrap()), None, &mut Trace::quiet()).await.unwrap());
        let r = db.run(&c, None, "s2", "SELECT TOP 60000 a.object_id FROM sys.all_columns a CROSS JOIN sys.all_columns b", false, 50_000).await.unwrap();
        assert!(r.results[0].truncated && r.results[0].rows.len() == 50_000);
        println!("cap run {} ms", r.elapsed_ms);
    }

    #[test]
    fn az_found_with_finder_path() {
        // SAFETY: test-only, single-threaded use of the env var before anything reads it.
        unsafe { std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin") };
        assert!(az_bin().is_absolute(), "{:?}", az_bin());
    }

    #[tokio::test]
    #[ignore]
    async fn row_limit_and_session_invariants() {
        let db = Db::default();
        let c = conn("localhost", "sql", Some("tempdb"));
        db.put("s", &c, None, db.connect(&c, Some(std::env::var("DBS_SA_PW").unwrap()), None, &mut Trace::quiet()).await.unwrap());
        let run = |sql: &'static str, ro: bool, max: usize| db.run(&c, None, "s", sql, ro, max);
        // Self-contained data: session temp tables (5k customers, 200k orders) in tempdb.
        run("SELECT TOP (5000) CAST(ROW_NUMBER() OVER (ORDER BY (SELECT 1)) AS int) Id, CAST(N'n' AS nvarchar(50)) Name INTO #Customers FROM sys.all_objects a CROSS JOIN sys.all_objects b;
             SELECT TOP (200000) CAST(ROW_NUMBER() OVER (ORDER BY (SELECT 1)) AS int) Id INTO #Orders FROM sys.all_objects a CROSS JOIN sys.all_objects b", false, 1000).await.unwrap();
        run("CREATE TABLE #keep(i int); INSERT #keep VALUES (1)", false, 1000).await.unwrap();
        // Server-side limit: 1000 of 200k rows, fast, and the session survives (temp table still there).
        let t0 = Instant::now();
        let r = run("SELECT * FROM #Orders", false, 1000).await.unwrap();
        assert!(r.results[0].truncated && r.results[0].rows.len() == 1000);
        println!("limited SELECT * of 200k: {} ms", t0.elapsed().as_millis());
        assert_eq!(run("SELECT COUNT(*) FROM #keep", false, 1000).await.unwrap().results[0].rows[0][0].as_deref(), Some("1"));
        // Limit must never leak into writes: this UPDATE must touch all 5000 rows.
        let r = run("UPDATE #Customers SET Name = Name", false, 1000).await.unwrap();
        assert_eq!(r.messages, ["(5000 rows affected by the last statement)"]);
        // Also not after a read that failed mid-batch (in-batch reset skipped, fallback reset runs).
        run("SET ROWCOUNT 0; SELECT * FROM #Orders; SELECT 1/0", false, 10).await.unwrap_err();
        assert_eq!(run("UPDATE #Customers SET Name = Name", false, 1000).await.unwrap().messages, ["(5000 rows affected by the last statement)"]);
        // Exactly at the limit is not truncated; limit 0 = no limit (up to HARD_CAP).
        assert!(!run("SELECT TOP (1000) * FROM #Orders", false, 1000).await.unwrap().results[0].truncated);
        assert_eq!(run("SELECT TOP (150000) Id FROM #Orders", false, 0).await.unwrap().results[0].rows.len(), 150_000);
        // Multiple result sets are each limited.
        let r = run("SELECT Id FROM #Orders; SELECT Id FROM #Customers", false, 5).await.unwrap();
        assert!(r.results.len() == 2 && r.results.iter().all(|x| x.rows.len() == 5 && x.truncated));
        // Read-only run inside the user's own open transaction is refused and the transaction survives.
        run("BEGIN TRAN; UPDATE #Customers SET Name = 'tx' WHERE Id = 1", false, 1000).await.unwrap();
        assert!(run("SELECT 1", true, 1000).await.unwrap_err().contains("open transaction"));
        assert_eq!(run("SELECT @@TRANCOUNT, (SELECT Name FROM #Customers WHERE Id = 1)", false, 1000).await.unwrap().results[0].rows[0], [Some("1".into()), Some("tx".into())]);
        run("ROLLBACK", false, 1000).await.unwrap();
        // Read-only mode: limited reads work, writes blocked, nothing left open.
        assert_eq!(run("SELECT * FROM #Orders", true, 50).await.unwrap().results[0].rows.len(), 50);
        assert!(run("UPDATE #Customers SET Name = 'x'", true, 1000).await.is_err());
        assert_eq!(run("SELECT @@TRANCOUNT", false, 1000).await.unwrap().results[0].rows[0][0].as_deref(), Some("0"));
        // Error line numbers match the editor despite our same-line prefixes.
        let e = run("SELECT 1\nSELECT * FROM nope", false, 1000).await.unwrap_err();
        assert!(e.contains("Line 2"), "{e}");
        let e = run("SELECT 1\nSELECT * FROM nope", true, 1000).await.unwrap_err();
        assert!(e.contains("Line 2"), "{e}");
    }
}
