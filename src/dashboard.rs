use crate::store::{NewTarget, Store};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinHandle;

type Shared = Arc<Mutex<Store>>;
type ApiError = (StatusCode, String);
fn db_error(e: rusqlite::Error) -> ApiError {
    tracing::error!(%e, "database operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Could not save or read monitoring data. Check the server log.".into(),
    )
}

pub async fn run(port: u16, database: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = std::path::Path::new(database)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let store = Store::open(database)?;
    store.reset_after_restart()?;
    let state = Arc::new(Mutex::new(store));
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let app = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route("/favicon.svg", get(|| async { ([(header::CONTENT_TYPE,"image/svg+xml")], r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="4" fill="#c6f27c"/><path d="M4 17h6l3-9 6 17 3-8h6" fill="none" stroke="#182113" stroke-width="2"/></svg>"##) }))
        .route("/api/status", get(status))
        .route("/api/targets", post(add))
        .route("/api/targets/{id}/toggle", post(toggle))
        .route("/api/targets/{id}", axum::routing::delete(remove))
        .route("/api/incidents.csv", get(export))
        .with_state(state.clone())
        .layer(middleware::from_fn(move |request, next| {
            protect(request, next, port)
        }));
    let scheduler = tokio::spawn(schedule(state));
    tracing::info!("NetCheck GUI: http://127.0.0.1:{port} (data: {database})");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    scheduler.abort();
    result?;
    Ok(())
}

async fn protect(request: axum::extract::Request, next: Next, port: u16) -> Response {
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !hosts.iter().any(|h| h == host) {
        return (StatusCode::FORBIDDEN, "Local access only").into_response();
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let valid = hosts
            .iter()
            .any(|h| origin.as_bytes() == format!("http://{h}").as_bytes());
        if !valid {
            return (StatusCode::FORBIDDEN, "Cross-origin requests are disabled").into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY,"default-src 'self'; script-src 'self'; style-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

async fn status(State(s): State<Shared>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = s.lock().unwrap();
    let open_incidents: u64 =
        s.db.query_row(
            "SELECT COUNT(*) FROM incidents WHERE ended_at IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    Ok(Json(serde_json::json!({
        "targets": s.targets().map_err(db_error)?,
        "incidents": s.incidents().map_err(db_error)?,
        "open_incidents": open_incidents,
        "server_time": crate::store::now()
    })))
}
async fn add(
    State(s): State<Shared>,
    Json(mut input): Json<NewTarget>,
) -> Result<(StatusCode, Json<crate::store::Target>), ApiError> {
    input.validate().map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let mut s = s.lock().unwrap();
    let targets = s.targets().map_err(db_error)?;
    if targets.len() >= 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Maximum 128 targets. Remove a target to add another.".into(),
        ));
    }
    if targets.iter().any(|t| t.address == input.address) {
        return Err((
            StatusCode::CONFLICT,
            "This IP address is already being monitored.".into(),
        ));
    }
    Ok((StatusCode::CREATED, Json(s.add(input).map_err(db_error)?)))
}
async fn toggle(State(s): State<Shared>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let mut s = s.lock().unwrap();
    let mut t = s
        .target(id)
        .map_err(|_| (StatusCode::NOT_FOUND, "Target not found".into()))?;
    let tx = s.db.transaction().map_err(db_error)?;
    tx.execute("UPDATE incidents SET ended_at=?,outcome='interrupted' WHERE target_id=? AND ended_at IS NULL",rusqlite::params![crate::store::now(),id]).map_err(db_error)?;
    t.enabled = !t.enabled;
    t.generation += 1;
    t.tracker = Default::default();
    t.status = if t.enabled { "pending" } else { "paused" }.into();
    t.latency_ms = None;
    t.error = None;
    tx.execute(
        "UPDATE targets SET data=? WHERE id=?",
        rusqlite::params![serde_json::to_string(&t).unwrap(), id],
    )
    .map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn remove(State(s): State<Shared>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let mut s = s.lock().unwrap();
    let tx = s.db.transaction().map_err(db_error)?;
    tx.execute("UPDATE incidents SET ended_at=?,outcome='interrupted' WHERE target_id=? AND ended_at IS NULL",rusqlite::params![crate::store::now(),id]).map_err(db_error)?;
    let changed = tx
        .execute("DELETE FROM targets WHERE id=?", [id])
        .map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    if changed == 0 {
        return Err((StatusCode::NOT_FOUND, "Target not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
fn csv_cell(value: &str) -> String {
    // Spreadsheet formulas must remain literal when opening an exported log.
    let prefix = if value.starts_with(['=', '+', '-', '@', '\t', '\r', '\n']) {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
async fn export(State(s): State<Shared>) -> Result<(HeaderMap, String), ApiError> {
    // Export the complete log; the GUI intentionally displays only the newest 1000.
    let s = s.lock().unwrap();
    let mut stmt=s.db.prepare("SELECT target_name,address,started_at,ended_at,outcome,reason FROM incidents ORDER BY id DESC").map_err(db_error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(db_error)?;
    let mut csv =
        "target,address,started_at_utc,ended_at_utc,duration_seconds,outcome,reason\r\n".to_owned();
    for row in rows {
        let (name, address, start, end, outcome, reason) = row.map_err(db_error)?;
        let date = |ms| {
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .to_rfc3339()
        };
        let duration = end
            .map(|e| format!("{:.3}", (e - start).max(0) as f64 / 1000.))
            .unwrap_or_default();
        csv.push_str(
            &[
                name,
                address,
                date(start),
                end.map(date).unwrap_or_default(),
                duration,
                outcome,
                reason,
            ]
            .iter()
            .map(|v| csv_cell(v))
            .collect::<Vec<_>>()
            .join(","),
        );
        csv.push_str("\r\n");
    }
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=netcheck-downtime.csv"
            .parse()
            .unwrap(),
    );
    Ok((h, csv))
}

async fn schedule(s: Shared) {
    let mut jobs: HashMap<i64, (u64, JoinHandle<()>)> = HashMap::new();
    loop {
        let targets = match s.lock().unwrap().targets() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(%e,"could not load targets");
                vec![]
            }
        };
        jobs.retain(|id, (generation, job)| {
            let keep = targets
                .iter()
                .any(|t| t.id == *id && t.enabled && t.generation == *generation)
                && !job.is_finished();
            if !keep {
                job.abort();
            }
            keep
        });
        for t in targets.into_iter().filter(|t| t.enabled) {
            jobs.entry(t.id).or_insert_with(|| {
                let s = s.clone();
                let generation = t.generation;
                let job = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(t.interval));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        interval.tick().await;
                        let result =
                            crate::checks::gateway::ping_gateway(t.address.parse().unwrap())
                                .await
                                .map(|d| d.as_secs_f64() * 1000.);
                        if let Err(e) =
                            s.lock()
                                .unwrap()
                                .record(t.id, generation, result, crate::store::now())
                        {
                            tracing::error!(%e,target=t.id,"failed to save probe");
                        }
                    }
                });
                (generation, job)
            });
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_formula_like_names_as_literals() {
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }
}
