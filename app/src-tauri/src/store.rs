//! Plain files under ~/.dbstudio so agents and humans can read and write them:
//!   connections.json      saved connections (no secrets; passwords live in the macOS Keychain)
//!   todos/<id>.sql        one queued query per file, header lines `-- key: value` then the SQL
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conn {
    pub id: String,
    pub name: String,
    pub server: String,
    pub port: u16,
    pub database: Option<String>,
    pub auth: String, // "sql" | "azure_cli"
    pub user: Option<String>,
    pub tenant: Option<String>,
    pub env: String, // local | dev | qa | prod
    pub read_only: bool,
    pub trust_cert: bool,
}

pub fn root() -> PathBuf {
    dirs::home_dir().expect("no home dir").join(".dbstudio")
}

fn conns_path() -> PathBuf {
    root().join("connections.json")
}

pub fn load_conns() -> Result<Vec<Conn>, String> {
    match fs::read_to_string(conns_path()) {
        Ok(s) => {
            let mut v: Vec<Conn> = serde_json::from_str(&s).map_err(|e| format!("connections.json: {e}"))?;
            for c in &mut v {
                // The file is hand/agent edited: "Prod", "production", typos all count as prod.
                c.env = match c.env.trim().to_lowercase().as_str() {
                    e @ ("local" | "dev" | "qa") => e.to_string(),
                    _ => "prod".into(),
                };
            }
            Ok(v)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save_conns(conns: &[Conn]) -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let tmp = conns_path().with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(conns).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(tmp, conns_path()).map_err(|e| e.to_string())
}

fn keychain(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("dbstudio", id).map_err(|e| e.to_string())
}

pub fn set_password(id: &str, pw: &str) -> Result<(), String> {
    keychain(id)?.set_password(pw).map_err(|e| e.to_string())
}

pub fn get_password(id: &str) -> Result<Option<String>, String> {
    // Dev rebuilds change the binary signature, so every rebuild re-triggers a blocking Keychain prompt.
    #[cfg(debug_assertions)]
    if let Ok(p) = std::env::var("DBSTUDIO_DEV_PASSWORD") {
        return Ok(Some(p));
    }
    match keychain(id)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The connection's own saved password, else one saved for another SQL-login connection with the
/// same server, port and user (connections pinned to one database share the server login).
pub fn password_for(c: &Conn) -> Result<Option<String>, String> {
    if let Some(p) = get_password(&c.id)? {
        return Ok(Some(p));
    }
    let same = |o: &&Conn| o.id != c.id && o.auth == "sql" && o.server.eq_ignore_ascii_case(&c.server) && o.port == c.port && o.user == c.user;
    for o in load_conns()?.iter().filter(same) {
        if let Some(p) = get_password(&o.id)? {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

pub fn delete_password(id: &str) {
    let _ = keychain(id).map(|k| k.delete_credential());
}

// ---------- todos ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub target: Option<String>,
    pub database: Option<String>,
    pub status: String,
    pub created: Option<String>,
    pub by: Option<String>,
    pub note: Option<String>,
    pub card: Option<String>, // work item / ticket id; typed with `execute prod` before a prod run
    pub sql: String,
}

#[derive(Deserialize)]
pub struct NewTodo {
    pub title: String,
    pub target: Option<String>,
    pub database: Option<String>,
    pub sql: String,
    pub by: Option<String>,
    pub card: Option<String>,
}

fn todos_dir() -> PathBuf {
    root().join("todos")
}

/// Ids are file stems; reject anything that could escape the todos dir.
fn todo_path(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.starts_with('.') || id.contains(['/', '\\']) {
        return Err(format!("bad todo id: {id}"));
    }
    Ok(todos_dir().join(format!("{id}.sql")))
}

/// Header = leading lines of the form `-- key: value`.
fn header_len(lines: &[&str]) -> usize {
    lines.iter().take_while(|l| parse_header(l).is_some()).count()
}

fn parse_header(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("--")?.trim();
    let (k, v) = rest.split_once(':')?;
    let k = k.trim().to_lowercase();
    k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-').then(|| (k, v.trim().to_string()))
}

pub fn parse_todo(id: &str, text: &str) -> Todo {
    let lines: Vec<&str> = text.lines().collect();
    let n = header_len(&lines);
    let mut t = Todo { id: id.into(), title: id.into(), status: "todo".into(), ..Default::default() };
    for (k, v) in lines[..n].iter().filter_map(|l| parse_header(l)) {
        let v2 = (!v.is_empty()).then(|| v.clone());
        match k.as_str() {
            "title" => t.title = v,
            "target" => t.target = v2,
            "database" => t.database = v2,
            "status" => t.status = v.to_lowercase(),
            "created" => t.created = v2,
            "by" => t.by = v2,
            "note" => t.note = v2,
            "card" => t.card = v2,
            _ => {}
        }
    }
    t.sql = lines[n..].join("\n").trim().to_string();
    t
}

/// Replace or add header `key` in place, keeping all other lines (including unknown keys).
fn set_header(text: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let n = header_len(&lines.iter().map(String::as_str).collect::<Vec<_>>());
    let new = format!("-- {key}: {}", value.replace(['\r', '\n'], " "));
    match lines[..n].iter().position(|l| parse_header(l).is_some_and(|(k, _)| k == key)) {
        Some(i) => lines[i] = new,
        None => lines.insert(n, new),
    }
    lines.join("\n") + "\n"
}

pub fn list_todos() -> Result<Vec<Todo>, String> {
    let rd = match fs::read_dir(todos_dir()) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let mut out: Vec<Todo> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let id = p.file_stem()?.to_str()?.to_string();
            (p.extension()? == "sql").then(|| parse_todo(&id, &fs::read_to_string(&p).unwrap_or_default()))
        })
        .collect();
    out.sort_by(|a, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
    Ok(out)
}

pub fn add_todo(t: NewTodo) -> Result<Todo, String> {
    fs::create_dir_all(todos_dir()).map_err(|e| e.to_string())?;
    let now = chrono::Local::now();
    let slug: String = t.title.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.split('-').filter(|s| !s.is_empty()).take(6).collect::<Vec<_>>().join("-");
    let id = format!("{}-{slug}", now.format("%Y%m%d-%H%M%S"));
    let mut text = format!("\n{}\n", t.sql.trim()); // blank line: SQL comments never read as headers
    for (k, v) in [
        ("card", t.card.unwrap_or_default()),
        ("by", t.by.unwrap_or_default()),
        ("created", now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)),
        ("status", "todo".into()),
        ("database", t.database.unwrap_or_default()),
        ("target", t.target.unwrap_or_default()),
        ("title", t.title),
    ] {
        text = format!("-- {k}: {}\n{text}", v.replace(['\r', '\n'], " "));
    }
    // create_new: two to-dos with the same title in the same second must not overwrite each other.
    for n in 1.. {
        let id = if n == 1 { id.clone() } else { format!("{id}-{n}") };
        match fs::OpenOptions::new().write(true).create_new(true).open(todo_path(&id)?) {
            Ok(mut f) => {
                std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| e.to_string())?;
                return Ok(parse_todo(&id, &text));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    unreachable!()
}

/// Fresh from disk (never trust a copy the UI polled seconds ago).
pub fn read_todo(id: &str) -> Result<Todo, String> {
    Ok(parse_todo(id, &fs::read_to_string(todo_path(id)?).map_err(|e| format!("to-do {id}: {e}"))?))
}

pub fn set_todo_status(id: &str, status: &str, note: Option<String>) -> Result<(), String> {
    if !matches!(status, "todo" | "done" | "failed") {
        return Err(format!("bad status: {status}"));
    }
    let p = todo_path(id)?;
    let mut text = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    text = set_header(&text, "status", status);
    if let Some(n) = note {
        text = set_header(&text, "note", &n);
    }
    fs::write(p, text).map_err(|e| e.to_string())
}

pub fn delete_todo(id: &str) -> Result<(), String> {
    fs::remove_file(todo_path(id)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_roundtrip() {
        let src = "-- title: Fix emails\n-- target: prod\n-- x-ticket: ABC-1\nUPDATE t SET a = 1 -- note: not a header\n";
        let t = parse_todo("a", src);
        assert_eq!((t.title.as_str(), t.target.as_deref(), t.status.as_str()), ("Fix emails", Some("prod"), "todo"));
        assert_eq!(t.sql, "UPDATE t SET a = 1 -- note: not a header");
        let out = set_header(&set_header(src, "status", "done"), "note", "3 rows\naffected");
        assert!(out.contains("-- x-ticket: ABC-1\n-- status: done\n-- note: 3 rows affected\nUPDATE"));
        assert_eq!(parse_todo("a", &out).status, "done");
        assert!(todo_path("../x").is_err() && todo_path("ok-1").is_ok());
    }
}
