use axum::{
    body::{Body, Bytes},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use rusqlite::{Connection, OpenFlags};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{mpsc, oneshot};

pub fn cell(value: &str) -> String {
    let prefix = if value.trim_start().starts_with(['=', '+', '-', '@'])
        || value.starts_with(['\t', '\r', '\n'])
    {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}

struct CsvStream(mpsc::Receiver<Result<Bytes, io::Error>>);
impl futures_core::Stream for CsvStream {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

#[derive(Clone, Copy)]
pub enum Report {
    Downtime,
    Activity,
}
pub async fn download(path: String, report: Report) -> rusqlite::Result<Response> {
    // A small channel bounds memory and applies backpressure without holding the monitor's mutex.
    let (sender, receiver) = mpsc::channel(4);
    let (ready, started) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let mut ready = Some(ready);
        let result = write_csv(&path, report, &sender, &mut ready);
        if let Err(error) = result {
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(error));
            } else {
                tracing::error!(%error,"CSV export failed after starting");
                let _ = sender.blocking_send(Err(io::Error::other("CSV export did not finish")));
            }
        }
    });
    started.await.map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::other(
            "CSV export worker stopped",
        )))
    })??;
    let mut response = (StatusCode::OK, Body::from_stream(CsvStream(receiver))).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_static(match report {
            Report::Downtime => "attachment; filename=netcheck-downtime.csv",
            Report::Activity => "attachment; filename=netcheck-activity.csv",
        }),
    );
    Ok(response)
}

fn write_csv(
    path: &str,
    report: Report,
    sender: &mpsc::Sender<Result<Bytes, io::Error>>,
    ready: &mut Option<oneshot::Sender<rusqlite::Result<()>>>,
) -> rusqlite::Result<()> {
    // An independent WAL reader keeps one consistent snapshot while probe writes continue.
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_secs(3))?;
    let transaction = db.transaction()?;
    let query=match report {
        Report::Downtime=>"SELECT target_name,address,started_at,ended_at,outcome,reason FROM incidents ORDER BY id DESC",
        Report::Activity=>"SELECT timestamp,target_name,address,kind,message,outage_started_at,duration_ms FROM activity ORDER BY timestamp DESC,id DESC",
    };
    let mut statement = transaction.prepare(query)?;
    let mut rows = statement.query([])?;
    if ready
        .take()
        .is_some_and(|ready| ready.send(Ok(())).is_err())
    {
        return Ok(());
    }
    let date = |ms| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|date| date.to_rfc3339())
            .unwrap_or_else(|| format!("Invalid timestamp: {ms}"))
    };
    let mut chunk=match report {Report::Downtime=>"target,address,started_at_utc,ended_at_utc,duration_seconds,outcome,reason\r\n",Report::Activity=>"timestamp_utc,target,address,event,what_happened,outage_started_at_utc,duration_seconds\r\n"}.to_owned();
    while let Some(row) = rows.next()? {
        if sender.is_closed() {
            return Ok(());
        }
        let fields: Vec<String> = match report {
            Report::Downtime => {
                let start: i64 = row.get(2)?;
                let end: Option<i64> = row.get(3)?;
                vec![
                    row.get(0)?,
                    row.get(1)?,
                    date(start),
                    end.map(date).unwrap_or_default(),
                    end.map(|end| {
                        format!("{:.3}", end.saturating_sub(start).max(0) as f64 / 1000.)
                    })
                    .unwrap_or_default(),
                    row.get(4)?,
                    row.get(5)?,
                ]
            }
            Report::Activity => {
                let timestamp: i64 = row.get(0)?;
                let start: Option<i64> = row.get(5)?;
                let duration: Option<i64> = row.get(6)?;
                vec![
                    date(timestamp),
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    start.map(date).unwrap_or_default(),
                    duration
                        .map(|ms| format!("{:.3}", ms as f64 / 1000.))
                        .unwrap_or_default(),
                ]
            }
        };
        chunk.push_str(
            &fields
                .iter()
                .map(|value| cell(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        chunk.push_str("\r\n");
        if chunk.len() >= 16 * 1024
            && sender
                .blocking_send(Ok(Bytes::from(std::mem::take(&mut chunk))))
                .is_err()
        {
            return Ok(());
        }
    }
    if !chunk.is_empty() {
        let _ = sender.blocking_send(Ok(Bytes::from(chunk)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formula_like_cells_remain_literal() {
        assert_eq!(cell("=1+1"), "\"'=1+1\"");
        assert_eq!(cell("  =1+1"), "\"'  =1+1\"");
        assert_eq!(cell("a\"b"), "\"a\"\"b\"");
    }
}
