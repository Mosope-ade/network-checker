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
use tokio::task::{AbortHandle, JoinSet};

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
    // Bind before opening/resetting storage: a failed launch must not mutate live incidents.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let local = listener.local_addr()?;
    let mut store = Store::open(database)?;
    store.reset_after_restart()?;
    let state = Arc::new(Mutex::new(store));
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
        .route("/api/targets/{id}/state", axum::routing::put(set_enabled))
        .route("/api/targets/{id}", axum::routing::delete(remove))
        .route("/api/incidents.csv", get(export))
        .with_state(state.clone())
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(move |request, next| {
            protect(request, next, local)
        }));
    let scheduler = tokio::spawn(schedule(state.clone()));
    tracing::info!("NetCheck GUI: http://{local} (data: {database})");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    scheduler.abort();
    let _ = scheduler.await;
    with_db(state, |s| s.reset_after_restart().map_err(db_error))
        .await
        .map_err(|(_, message)| std::io::Error::other(message))?;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=terminate.recv()=>{} }
            }
            Err(error) => {
                tracing::error!(%error,"Could not register termination handler");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
async fn with_db<T: Send + 'static>(
    s: Shared,
    operation: impl FnOnce(&mut Store) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || {
        let mut store = s.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Monitoring storage is unavailable; restart NetCheck.".into(),
            )
        })?;
        operation(&mut store)
    })
    .await
    .map_err(|error| {
        tracing::error!(%error,"Database worker failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database worker failed; check server log.".into(),
        )
    })?
}
fn target_error(error: rusqlite::Error) -> ApiError {
    if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
        (StatusCode::NOT_FOUND, "Target not found".into())
    } else {
        db_error(error)
    }
}
async fn protect(
    request: axum::extract::Request,
    next: Next,
    local: std::net::SocketAddr,
) -> Response {
    let hosts = [local.to_string(), format!("localhost:{}", local.port())];
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
    with_db(s, |s| {
        let open_incidents: i64 =
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
    })
    .await
}
async fn add(
    State(s): State<Shared>,
    Json(mut input): Json<NewTarget>,
) -> Result<(StatusCode, Json<crate::store::Target>), ApiError> {
    input.validate().map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    with_db(s, move |s| {
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
    })
    .await
}
#[derive(serde::Deserialize)]
struct Enabled {
    enabled: bool,
}
async fn set_enabled(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(input): Json<Enabled>,
) -> Result<StatusCode, ApiError> {
    with_db(s, move |s| {
        s.set_enabled(id, input.enabled).map_err(target_error)?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}
async fn remove(State(s): State<Shared>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    with_db(s,move |s| {
        let t=s.target(id).map_err(target_error)?;
        let tx=s.db.transaction().map_err(db_error)?;
        tx.execute("UPDATE incidents SET ended_at=MAX(started_at,?),outcome='interrupted' WHERE target_id=? AND ended_at IS NULL",rusqlite::params![t.last_check.unwrap_or_else(crate::store::now),id]).map_err(db_error)?;
        tx.execute("DELETE FROM targets WHERE id=?",[id]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;Ok(StatusCode::NO_CONTENT)
    }).await
}
fn csv_cell(value: &str) -> String {
    // Spreadsheet formulas must remain literal when opening an exported log.
    let prefix = if value.trim_start().starts_with(['=', '+', '-', '@'])
        || value.starts_with(['\t', '\r', '\n'])
    {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
async fn export(State(s): State<Shared>) -> Result<(HeaderMap, String), ApiError> {
    // Export the complete log; the GUI intentionally displays only the newest 1000.
    with_db(s, |s| {
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
            chrono::DateTime::from_timestamp_millis(ms).map(|date|date.to_rfc3339()).unwrap_or_else(||format!("Invalid timestamp: {ms}"))
        };
        let duration = end
            .map(|e| format!("{:.3}", e.saturating_sub(start).max(0) as f64 / 1000.))
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
    }).await
}

async fn schedule(s: Shared) {
    // Dropping the JoinSet on cancellation aborts every child probe task.
    let mut workers = JoinSet::new();
    let mut jobs: HashMap<i64, (u64, AbortHandle)> = HashMap::new();
    loop {
        while let Some(result) = workers.try_join_next() {
            if let Err(error) = result {
                if !error.is_cancelled() {
                    tracing::error!(%error,"Probe task failed");
                }
            }
        }
        let targets = match with_db(s.clone(), |s| s.targets().map_err(db_error)).await {
            Ok(targets) => targets,
            Err(_) => {
                workers.abort_all();
                jobs.clear();
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
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
        for target in targets.into_iter().filter(|t| t.enabled) {
            jobs.entry(target.id).or_insert_with(|| {
                let state = s.clone();
                let generation = target.generation;
                let job = workers.spawn(async move {
                    let address = match target.address.parse() {
                        Ok(address) => address,
                        Err(error) => {
                            tracing::error!(%error,"Invalid persisted target address");
                            return;
                        }
                    };
                    let mut interval = tokio::time::interval(Duration::from_secs(target.interval));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        interval.tick().await;
                        let result = crate::ping::probe(address).await;
                        if let Err((_, error)) = with_db(state.clone(), move |store| {
                            store
                                .record(target.id, generation, result, crate::store::now())
                                .map_err(db_error)
                        })
                        .await
                        {
                            tracing::error!(%error,target=target.id,"Could not save probe");
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
        assert_eq!(csv_cell("  =1+1"), "\"'  =1+1\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }
}
