use crate::store::Target;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Event {
    pub id: i64,
    pub timestamp: i64,
    pub target_name: String,
    pub address: String,
    pub kind: String,
    pub message: String,
    pub outage_started_at: Option<i64>,
    pub duration_ms: Option<i64>,
    #[serde(skip)]
    pub source: Option<String>,
    #[serde(skip)]
    pub target_id: i64,
}
impl Event {
    pub fn new(target: &Target, timestamp: i64, kind: &str, message: impl Into<String>) -> Self {
        Self {
            id: 0,
            timestamp,
            target_id: target.id,
            target_name: target.name.clone(),
            address: target.address.clone(),
            kind: kind.into(),
            message: message.into(),
            outage_started_at: None,
            duration_ms: None,
            source: None,
        }
    }
    pub fn outage(mut self, start: i64, duration: Option<i64>) -> Self {
        self.outage_started_at = Some(start);
        self.duration_ms = duration;
        self
    }
    pub fn source(mut self, key: String) -> Self {
        self.source = Some(key);
        self
    }
}
pub fn write(db: &Connection, event: Event) -> rusqlite::Result<()> {
    db.execute("INSERT OR IGNORE INTO activity(timestamp,target_id,target_name,address,kind,message,outage_started_at,duration_ms,source) VALUES(?,?,?,?,?,?,?,?,?)",params![event.timestamp,event.target_id,event.target_name,event.address,event.kind,event.message,event.outage_started_at,event.duration_ms,event.source])?;
    Ok(())
}
pub fn list(db: &Connection) -> rusqlite::Result<Vec<Event>> {
    let mut statement=db.prepare("SELECT id,timestamp,target_id,target_name,address,kind,message,outage_started_at,duration_ms FROM activity ORDER BY timestamp DESC,id DESC LIMIT 1000")?;
    let rows = statement.query_map([], |row| {
        Ok(Event {
            id: row.get(0)?,
            timestamp: row.get(1)?,
            target_id: row.get(2)?,
            target_name: row.get(3)?,
            address: row.get(4)?,
            kind: row.get(5)?,
            message: row.get(6)?,
            outage_started_at: row.get(7)?,
            duration_ms: row.get(8)?,
            source: None,
        })
    })?;
    rows.collect()
}
// Close each incident together with an explanatory event, preserving the observed cutoff time.
pub fn interrupt(db: &Connection, target: &Target, at: i64, reason: &str) -> rusqlite::Result<()> {
    let mut statement =
        db.prepare("SELECT id,started_at FROM incidents WHERE target_id=? AND ended_at IS NULL")?;
    let incidents = statement
        .query_map([target.id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, start) in incidents {
        let end = at.max(start);
        db.execute(
            "UPDATE incidents SET ended_at=?,outcome='interrupted' WHERE id=?",
            params![end, id],
        )?;
        write(
            db,
            Event::new(target, end, "interrupted", reason)
                .outage(start, Some(end.saturating_sub(start)))
                .source(format!("incident:{id}:end")),
        )?;
    }
    Ok(())
}
