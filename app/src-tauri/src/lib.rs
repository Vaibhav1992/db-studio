mod db;
#[cfg(debug_assertions)]
mod devbridge;
mod sqlguard;
mod store;

use db::{Db, QueryResult};
use serde::Serialize;
use store::{Conn, NewTodo, Todo};
use std::collections::BTreeMap;
use tauri::State;

type R<T> = Result<T, String>;

fn conn(id: &str) -> R<Conn> {
    store::load_conns()?.into_iter().find(|c| c.id == id).ok_or_else(|| format!("Unknown connection {id}"))
}

#[tauri::command]
fn list_connections() -> R<Vec<Conn>> {
    store::load_conns()
}

#[tauri::command]
fn save_connection(mut conn: Conn, password: Option<String>) -> R<Conn> {
    let mut all = store::load_conns()?;
    if conn.id.is_empty() {
        conn.id = uuid::Uuid::new_v4().to_string();
    }
    if let Some(pw) = password.filter(|p| !p.is_empty()) {
        store::set_password(&conn.id, &pw)?;
    }
    match all.iter_mut().find(|c| c.id == conn.id) {
        Some(c) => *c = conn.clone(),
        None => all.push(conn.clone()),
    }
    store::save_conns(&all)?;
    Ok(conn)
}

#[tauri::command]
fn delete_connection(id: String) -> R<()> {
    let mut all = store::load_conns()?;
    all.retain(|c| c.id != id);
    store::delete_password(&id);
    store::save_conns(&all)
}

#[tauri::command]
async fn test_connection(db: State<'_, Db>, conn: Conn, password: Option<String>) -> R<String> {
    let mut client = db.connect(&conn, password.filter(|p| !p.is_empty()), None).await?;
    let row = client.simple_query("SELECT @@VERSION").await.map_err(db::fmt_err)?.into_row().await.map_err(db::fmt_err)?;
    Ok(row.and_then(|r| r.get::<&str, _>(0).map(|s| s.lines().next().unwrap_or("").to_string())).unwrap_or_default())
}

#[tauri::command]
async fn list_databases(db: State<'_, Db>, conn_id: String) -> R<Vec<String>> {
    let c = conn(&conn_id)?;
    let rows = db.meta(&c, None, "SELECT name FROM sys.databases ORDER BY CASE WHEN database_id <= 4 THEN 1 ELSE 0 END, name", &[]).await?;
    Ok(rows.into_iter().filter_map(|mut r| r.remove(0)).collect())
}

#[derive(Serialize)]
struct DbObject {
    schema: String,
    name: String,
    kind: &'static str,
}

#[tauri::command]
async fn list_objects(db: State<'_, Db>, conn_id: String, database: String) -> R<Vec<DbObject>> {
    let c = conn(&conn_id)?;
    let sql = "SELECT s.name, o.name, RTRIM(o.type) FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id \
               WHERE o.type IN ('U','V','P','FN','IF','TF') AND o.is_ms_shipped = 0 ORDER BY s.name, o.name";
    let rows = db.meta(&c, Some(&database), sql, &[]).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let kind = match r[2].as_deref() {
                Some("U") => "table",
                Some("V") => "view",
                Some("P") => "procedure",
                _ => "function",
            };
            DbObject { schema: r[0].clone().unwrap_or_default(), name: r[1].clone().unwrap_or_default(), kind }
        })
        .collect())
}

/// Tables and views with their columns for editor autocomplete: { schema: { table: [columns] } }.
/// One round trip per database; the UI caches it until "Refresh IntelliSense".
#[tauri::command]
async fn list_schema(db: State<'_, Db>, conn_id: String, database: String) -> R<BTreeMap<String, BTreeMap<String, Vec<String>>>> {
    let c = conn(&conn_id)?;
    let sql = "SELECT s.name, o.name, c.name FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id \
               JOIN sys.columns c ON c.object_id = o.object_id \
               WHERE o.type IN ('U','V','IF','TF') AND o.is_ms_shipped = 0 ORDER BY s.name, o.name, c.column_id";
    let mut out: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for mut r in db.meta(&c, Some(&database), sql, &[]).await? {
        let (col, table, schema) = (r.pop().flatten(), r.pop().flatten(), r.pop().flatten());
        if let (Some(s), Some(t), Some(col)) = (schema, table, col) {
            out.entry(s).or_default().entry(t).or_default().push(col);
        }
    }
    Ok(out)
}

#[derive(Serialize)]
struct Column {
    name: String,
    r#type: String,
    nullable: bool,
}

#[tauri::command]
async fn list_columns(db: State<'_, Db>, conn_id: String, database: String, schema: String, table: String) -> R<Vec<Column>> {
    let c = conn(&conn_id)?;
    let sql = "SELECT c.name, t.name, c.max_length, c.precision, c.scale, c.is_nullable FROM sys.columns c \
               JOIN sys.types t ON t.user_type_id = c.user_type_id WHERE c.object_id = OBJECT_ID(@P1) ORDER BY c.column_id";
    let full = format!("[{}].[{}]", schema.replace(']', "]]"), table.replace(']', "]]"));
    let rows = db.meta(&c, Some(&database), sql, &[&full]).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let s = |i: usize| r[i].clone().unwrap_or_default();
            let n = |i: usize| s(i).parse::<i32>().unwrap_or(0);
            let t = s(1);
            let ty = match t.as_str() {
                "varchar" | "char" | "varbinary" | "binary" => format!("{t}({})", if n(2) == -1 { "max".into() } else { n(2).to_string() }),
                "nvarchar" | "nchar" => format!("{t}({})", if n(2) == -1 { "max".into() } else { (n(2) / 2).to_string() }),
                "decimal" | "numeric" => format!("{t}({},{})", n(3), n(4)),
                "datetime2" | "time" | "datetimeoffset" => format!("{t}({})", n(4)),
                _ => t,
            };
            Column { name: s(0), r#type: ty, nullable: s(5) == "1" }
        })
        .collect())
}

#[tauri::command]
async fn run_query(
    db: State<'_, Db>,
    conn_id: String,
    database: Option<String>,
    session: String,
    sql: String,
    read_only: bool,
    max_rows: Option<usize>,
    confirm: Option<String>,
) -> R<QueryResult> {
    let c = conn(&conn_id)?;
    let read_only = read_only || c.read_only;
    if c.env == "prod" && !read_only && !prod_confirmed(confirm.as_deref()) {
        if let Some(why) = sqlguard::split_batches(&sql).iter().find_map(|b| sqlguard::write_reason(b)) {
            return Err(format!("CONFIRM_WRITE:This is a write ({why}) on PRODUCTION \"{}\".", c.name));
        }
    }
    let database = database.filter(|d| !d.is_empty());
    db.run(&c, database.as_deref(), &session, &sql, read_only, max_rows.unwrap_or(100)).await
}

/// Run a queued to-do: re-read the file, resolve the target by exact connection name, run, record result.
/// A prod write needs the typed phrase `execute prod <card>` (or `execute prod` when the to-do has no card).
#[tauri::command]
async fn run_todo(db: State<'_, Db>, id: String, confirm: Option<String>) -> R<QueryResult> {
    let t = store::read_todo(&id)?;
    let target = t.target.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or("To-do has no target connection.")?;
    // Exact name only: an env name like "prod" could match several servers.
    let mut found: Vec<Conn> = store::load_conns()?.into_iter().filter(|c| c.name.trim().eq_ignore_ascii_case(target)).collect();
    if found.len() != 1 {
        return Err(format!("Target \"{target}\" matches {} connections; it must match exactly one connection name.", found.len()));
    }
    let c = found.remove(0);
    let phrase = match t.card.as_deref() {
        Some(card) => format!("execute prod {card}"),
        None => "execute prod".into(),
    };
    let confirm = confirm.filter(|p| p.trim().eq_ignore_ascii_case(&phrase));
    let r = run_query(db, c.id.clone(), t.database.clone(), format!("todo:{id}"), t.sql.clone(), false, Some(100), confirm).await;
    let place = format!("{}/{}", c.name, t.database.as_deref().or(c.database.as_deref()).unwrap_or("default"));
    let when = chrono::Local::now().format("%Y-%m-%d %H:%M");
    match &r {
        Err(e) if e.starts_with("CONFIRM_WRITE:") => return Err(format!("{e} Type \"{phrase}\" to run.")),
        Ok(res) => {
            // Messages plus a short view of each result: `Updated=5000` for one small row, else `N rows`.
            let mut parts = res.messages.clone();
            for x in &res.results {
                parts.push(match x.rows.as_slice() {
                    [row] if row.len() <= 3 => x.columns.iter().zip(row).map(|(c, v)| format!("{c}={}", v.as_deref().unwrap_or("NULL"))).collect::<Vec<_>>().join(", "),
                    rows => format!("{} rows", rows.len()),
                });
            }
            let what = parts.join("; ");
            store::set_todo_status(&id, "done", Some(format!("{what} · {place} · {when}")))?;
        }
        Err(e) => {
            let one: String = e.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect();
            store::set_todo_status(&id, "failed", Some(format!("{one} · {place} · {when}")))?;
        }
    }
    r
}

/// Prod writes need the typed phrase `execute prod` (the UI may require a card id after it).
fn prod_confirmed(confirm: Option<&str>) -> bool {
    confirm.is_some_and(|p| p.trim().to_lowercase().starts_with("execute prod"))
}

/// Source of a procedure, view, function or trigger, as `CREATE OR ALTER` so it can be edited and re-run.
#[tauri::command]
async fn get_definition(db: State<'_, Db>, conn_id: String, database: String, schema: String, name: String) -> R<String> {
    let c = conn(&conn_id)?;
    let full = format!("[{}].[{}]", schema.replace(']', "]]"), name.replace(']', "]]"));
    let rows = db.meta(&c, Some(&database), "SELECT OBJECT_DEFINITION(OBJECT_ID(@P1))", &[&full]).await?;
    let def = rows.into_iter().next().and_then(|mut r| r.remove(0)).ok_or("No definition (encrypted, a table, or no VIEW DEFINITION permission).")?;
    Ok(to_create_or_alter(&def))
}

/// Swap the first `CREATE` keyword (outside comments) for `CREATE OR ALTER`.
fn to_create_or_alter(def: &str) -> String {
    let w = sqlguard::words(def);
    if w.first().map(String::as_str) != Some("CREATE") || w.get(1).map(String::as_str) == Some("OR") {
        return def.to_string();
    }
    let lower = def.to_ascii_lowercase(); // ASCII keeps byte offsets aligned with def
    let mut from = 0;
    while let Some(i) = lower[from..].find("create").map(|i| i + from) {
        if sqlguard::words(&def[..i + 6]) == ["CREATE"] {
            return format!("{}CREATE OR ALTER{}", &def[..i], &def[i + 6..]);
        }
        from = i + 6;
    }
    def.to_string()
}

#[tauri::command]
fn cancel_query(db: State<'_, Db>, session: String) {
    db.cancel(&session);
}

#[tauri::command]
fn close_session(db: State<'_, Db>, session: String) {
    db.cancel(&session);
    db.close(&session);
}

#[tauri::command]
fn list_todos() -> R<Vec<Todo>> {
    store::list_todos()
}

#[tauri::command]
fn add_todo(todo: NewTodo) -> R<Todo> {
    store::add_todo(todo)
}

#[tauri::command]
fn set_todo_status(id: String, status: String, note: Option<String>) -> R<()> {
    store::set_todo_status(&id, &status, note)
}

#[tauri::command]
fn delete_todo(id: String) -> R<()> {
    store::delete_todo(&id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Db::default())
        // Default macOS menu binds Cmd+W to "Close Window"; we want Cmd+W to close a query tab.
        .setup(|app| {
            use tauri::menu::{Menu, PredefinedMenuItem as P, Submenu};
            let h = app.handle();
            let menu = Menu::with_items(h, &[
                &Submenu::with_items(h, "DB Studio", true, &[&P::about(h, None, None)?, &P::separator(h)?, &P::hide(h, None)?, &P::quit(h, None)?])?,
                &Submenu::with_items(h, "Edit", true, &[
                    &P::undo(h, None)?, &P::redo(h, None)?, &P::separator(h)?,
                    &P::cut(h, None)?, &P::copy(h, None)?, &P::paste(h, None)?, &P::select_all(h, None)?,
                ])?,
            ])?;
            app.set_menu(menu)?;
            #[cfg(debug_assertions)]
            if std::env::var("DBSTUDIO_DEV_BRIDGE").as_deref() == Ok("1") {
                devbridge::start(app.handle().clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_connections,
            save_connection,
            delete_connection,
            test_connection,
            list_databases,
            list_objects,
            list_columns,
            list_schema,
            run_query,
            get_definition,
            run_todo,
            cancel_query,
            close_session,
            list_todos,
            add_todo,
            set_todo_status,
            delete_todo
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_or_alter_and_phrase() {
        assert_eq!(super::to_create_or_alter("-- create x\ncreate PROCEDURE p AS SELECT 1"), "-- create x\nCREATE OR ALTER PROCEDURE p AS SELECT 1");
        assert_eq!(super::to_create_or_alter("CREATE OR ALTER VIEW v AS SELECT 1"), "CREATE OR ALTER VIEW v AS SELECT 1");
        assert!(super::prod_confirmed(Some(" Execute PROD ABC-1")) && !super::prod_confirmed(Some("yes")) && !super::prod_confirmed(None));
    }
}
