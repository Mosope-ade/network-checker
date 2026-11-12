use crate::store::{NewTarget, Store};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
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
    if let rusqlite::Error::SqliteFailure(error, _) = &e {
        if error.code == rusqlite::ErrorCode::DiskFull {
            return (
                StatusCode::INSUFFICIENT_STORAGE,
                "Monitoring storage is full.".into(),
            );
        }
        if matches!(
            error.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ) {
            return (StatusCode::LOCKED, "Monitoring storage is busy.".into());
        }
    }
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
    if store.db.path().is_none_or(|path| path.is_empty()) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"Choose a database file with --database so downtime logs survive restarts; memory-only databases are not supported.").into());
    }
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
        .route("/api/activity.csv",get(export_activity))
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
    with_db(state, |s| s.stop().map_err(db_error))
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
    let state_request = request.uri().path().ends_with("/state");
    let hosts = [local.to_string(), format!("localhost:{}", local.port())];
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !hosts.iter().any(|h| h == host) {
        return secure(error_response(
            StatusCode::FORBIDDEN,
            "Open NetCheck using the local URL printed in the terminal.",
        ));
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let valid = hosts
            .iter()
            .any(|h| origin.as_bytes() == format!("http://{h}").as_bytes());
        if !valid {
            return secure(error_response(StatusCode::FORBIDDEN,"This request came from a different site. Open NetCheck using its local URL and try again."));
        }
    }
    let response = next.run(request).await;
    secure(normalize_error(response, state_request).await)
}
fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":status.as_u16(),"message":message}})),
    )
        .into_response()
}
async fn normalize_error(response: Response, state_request: bool) -> Response {
    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, 16 * 1024)
        .await
        .unwrap_or_default();
    let original = String::from_utf8_lossy(&bytes);
    let message=match status {
        StatusCode::INSUFFICIENT_STORAGE => "There is not enough disk space to save monitoring data. Free space on the database drive, then try again.",
        StatusCode::LOCKED => "The monitoring database is busy. Wait a moment and try again. Check whether another program is holding it open.",
        StatusCode::NOT_FOUND => "This target or page no longer exists. Refresh the page to see the current targets.",
        StatusCode::METHOD_NOT_ALLOWED => "This operation is not supported. Refresh the page to load the current app and try again.",
        StatusCode::PAYLOAD_TOO_LARGE => "This request is too large. Add fewer targets at a time and keep names under 80 characters.",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "The request format was not accepted. Refresh the page and try again.",
        StatusCode::UNPROCESSABLE_ENTITY if state_request => "The monitoring state was not valid. Refresh the page and try Pause or Resume again.",
        StatusCode::UNPROCESSABLE_ENTITY => "Check that each target has a name and IP address, and that the interval and thresholds are whole numbers.",
        StatusCode::BAD_REQUEST if original.starts_with("Failed") || original.starts_with("Invalid URL") => "The request could not be read. Refresh the page and check the values before trying again.",
        _ if status.is_server_error() => "NetCheck could not read or save local monitoring data. Try again. If the problem continues, check the server log before restarting.",
        _ if original.trim().is_empty() => "The request could not be completed. Refresh the page and try again.",
        _ => original.as_ref(),
    };
    let mut response = error_response(status, message);
    if let Some(allow) = parts.headers.get(header::ALLOW) {
        response.headers_mut().insert(header::ALLOW, allow.clone());
    }
    response
}
fn secure(mut response: Response) -> Response {
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
        "activity": crate::activity::list(&s.db).map_err(db_error)?,
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
        crate::activity::interrupt(&tx,&t,t.last_check.unwrap_or_else(crate::store::now),"The target was removed before recovery was confirmed.").map_err(db_error)?;
        crate::activity::write(&tx,crate::activity::Event::new(&t,crate::store::now(),"removed","The target was removed by the user. Its activity and downtime history are retained.")).map_err(db_error)?;
        tx.execute("DELETE FROM targets WHERE id=?",[id]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;Ok(StatusCode::NO_CONTENT)
    }).await
}
async fn export(State(s): State<Shared>) -> Result<Response, ApiError> {
    export_report(s, crate::csv::Report::Downtime).await
}
async fn export_activity(State(s): State<Shared>) -> Result<Response, ApiError> {
    export_report(s, crate::csv::Report::Activity).await
}
async fn export_report(s: Shared, report: crate::csv::Report) -> Result<Response, ApiError> {
    let path = with_db(s, |store| {
        store
            .db
            .path()
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The export database is unavailable.".into(),
                )
            })
    })
    .await?;
    crate::csv::download(path, report).await.map_err(db_error)
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
                        let observed_at = crate::store::now();
                        if let Err((_, error)) = with_db(state.clone(), move |store| {
                            store
                                .record(target.id, generation, result, observed_at)
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
    fn disk_full_and_database_busy_have_distinct_recoverable_errors() {
        let full = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            None,
        );
        assert_eq!(db_error(full).0, StatusCode::INSUFFICIENT_STORAGE);
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        );
        assert_eq!(db_error(busy).0, StatusCode::LOCKED);
    }
}
