//! Debug builds only, opt-in with DBSTUDIO_DEV_BRIDGE=1: serves the Tauri commands over
//! http://127.0.0.1:1421/<command> so the UI can be tested end to end in a normal browser at
//! http://localhost:1420. Only connections to a loopback host are visible or usable through it.
use super::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::Manager;
use tiny_http::{Header, Method, Response, Server};

const ORIGIN: &str = "http://localhost:1420";

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let server = Server::http("127.0.0.1:1421").expect("dev bridge: port 1421 busy");
        for mut req in server.incoming_requests() {
            let app = app.clone();
            // One thread per request so cancel_query can run while run_query is in flight.
            std::thread::spawn(move || {
                let h = |k: &str, v: &str| Header::from_bytes(k, v).unwrap();
                let cors = [h("Access-Control-Allow-Origin", ORIGIN), h("Access-Control-Allow-Headers", "content-type")];
                // Browsers always send Origin on cross-origin requests; this blocks other web pages.
                let origin_ok = req.headers().iter().any(|x| x.field.equiv("Origin") && x.value.as_str() == ORIGIN);
                let (code, text) = if !origin_ok {
                    (403, "bad origin".to_string())
                } else if req.method() == &Method::Options {
                    (204, String::new())
                } else {
                    let cmd = req.url().trim_start_matches('/').to_string();
                    let mut body = String::new();
                    let _ = req.as_reader().read_to_string(&mut body);
                    let args: Value = serde_json::from_str(&body).unwrap_or(json!({}));
                    match tauri::async_runtime::block_on(dispatch(&app, &cmd, &args)) {
                        Ok(v) => (200, v.to_string()),
                        Err(e) => (400, e),
                    }
                };
                let mut resp = Response::from_string(text).with_status_code(code);
                for c in cors {
                    resp.add_header(c);
                }
                let _ = req.respond(resp);
            });
        }
    });
}

fn a<T: DeserializeOwned>(v: &Value, k: &str) -> R<T> {
    serde_json::from_value(v.get(k).cloned().unwrap_or(Value::Null)).map_err(|e| format!("arg {k}: {e}"))
}

fn j<T: Serialize>(x: T) -> R<Value> {
    serde_json::to_value(x).map_err(|e| e.to_string())
}

/// Tests must never reach a remote server: allow loopback hosts only, whatever the env label says.
fn local(c: &Conn) -> R<()> {
    let host = c.server.trim().to_lowercase();
    if matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | ".") { Ok(()) } else { Err(format!("dev bridge: \"{}\" is not on localhost", c.name)) }
}

async fn dispatch(app: &tauri::AppHandle, cmd: &str, v: &Value) -> R<Value> {
    let db = || app.state::<Db>();
    let conn_id = || -> R<String> {
        let id: String = a(v, "connId")?;
        local(&conn(&id)?)?;
        Ok(id)
    };
    match cmd {
        "list_connections" => j(list_connections()?.into_iter().filter(|c| local(c).is_ok()).collect::<Vec<_>>()),
        "save_connection" => {
            let c: Conn = a(v, "conn")?;
            local(&c)?;
            j(save_connection(c, a(v, "password")?)?)
        }
        "delete_connection" => {
            let id: String = a(v, "id")?;
            local(&conn(&id)?)?;
            j(delete_connection(id)?)
        }
        "test_connection" => {
            let c: Conn = a(v, "conn")?;
            local(&c)?;
            j(test_connection(db(), c, a(v, "password")?).await?)
        }
        "list_databases" => j(list_databases(db(), conn_id()?).await?),
        "list_objects" => j(list_objects(db(), conn_id()?, a(v, "database")?).await?),
        "list_columns" => j(list_columns(db(), conn_id()?, a(v, "database")?, a(v, "schema")?, a(v, "table")?).await?),
        "list_schema" => j(list_schema(db(), conn_id()?, a(v, "database")?).await?),
        "get_definition" => j(get_definition(db(), conn_id()?, a(v, "database")?, a(v, "schema")?, a(v, "name")?).await?),
        "run_query" => j(run_query(
            db(), conn_id()?, a(v, "database")?, a(v, "session")?, a(v, "sql")?, a(v, "readOnly")?, a(v, "maxRows")?, a(v, "confirm")?,
        )
        .await?),
        "run_todo" => {
            // Same exact-name resolution as the command; refuse unless that connection is on localhost.
            let id: String = a(v, "id")?;
            let t = store::read_todo(&id)?;
            let name = t.target.unwrap_or_default();
            let c = store::load_conns()?.into_iter().find(|c| c.name.trim().eq_ignore_ascii_case(name.trim())).ok_or("dev bridge: unknown target")?;
            local(&c)?;
            j(run_todo(db(), id, a(v, "confirm")?).await?)
        }
        "cancel_query" => j(cancel_query(db(), a(v, "session")?)),
        "close_session" => j(close_session(db(), a(v, "session")?)),
        "list_todos" => j(list_todos()?),
        "add_todo" => j(add_todo(a(v, "todo")?)?),
        "set_todo_status" => j(set_todo_status(a(v, "id")?, a(v, "status")?, a(v, "note")?)?),
        "delete_todo" => j(delete_todo(a(v, "id")?)?),
        _ => Err(format!("dev bridge: unknown command {cmd}")),
    }
}
