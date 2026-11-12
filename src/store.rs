use crate::tracker::{Tracker, Transition};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub enabled: bool,
    pub interval: u64,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub status: String,
    pub last_check: Option<i64>,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
    pub sent: u64,
    pub received: u64,
    pub history: Vec<Option<f64>>,
    pub tracker: Tracker,
    pub generation: u64,
}

#[derive(Debug, Deserialize)]
pub struct NewTarget {
    pub name: String,
    pub address: String,
    pub interval: u64,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
}

impl NewTarget {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_owned();
        self.address = self
            .address
            .trim()
            .parse::<IpAddr>()
            .map_err(|_| "Enter a valid IPv4 or IPv6 address (one host per target).".to_string())?
            .to_string();
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err("Name must contain 1–80 characters.".into());
        }
        if !(1..=3600).contains(&self.interval)
            || !(1..=20).contains(&self.failure_threshold)
            || !(1..=20).contains(&self.recovery_threshold)
        {
            return Err("Interval must be 1–3600 seconds; thresholds must be 1–20.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct Incident {
    pub id: i64,
    pub target_name: String,
    pub address: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub outcome: String,
    pub reason: String,
}

pub struct Store {
    pub db: Connection,
}
impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch("PRAGMA journal_mode=WAL;
          CREATE TABLE IF NOT EXISTS targets(id INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS incidents(id INTEGER PRIMARY KEY, target_id INTEGER NOT NULL,
            target_name TEXT NOT NULL, address TEXT NOT NULL, started_at INTEGER NOT NULL,
            ended_at INTEGER, outcome TEXT NOT NULL, reason TEXT NOT NULL);
          CREATE INDEX IF NOT EXISTS incident_target ON incidents(target_id, ended_at);")?;
        Ok(Self { db })
    }
    pub fn targets(&self) -> rusqlite::Result<Vec<Target>> {
        let mut stmt = self.db.prepare("SELECT data FROM targets ORDER BY id")?;
        let result = stmt
            .query_map([], |r| {
                let data: String = r.get(0)?;
                serde_json::from_str(&data).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })?
            .collect();
        result
    }
    pub fn target(&self, id: i64) -> rusqlite::Result<Target> {
        let data: String = self
            .db
            .query_row("SELECT data FROM targets WHERE id=?", [id], |r| r.get(0))?;
        serde_json::from_str(&data).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    }
    pub fn save(&self, target: &Target) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE targets SET data=? WHERE id=?",
            params![serde_json::to_string(target).unwrap(), target.id],
        )?;
        Ok(())
    }
    pub fn add(&mut self, input: NewTarget) -> rusqlite::Result<Target> {
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO targets(data) VALUES('{}')", [])?;
        let t = Target {
            id: tx.last_insert_rowid(),
            name: input.name,
            address: input.address,
            enabled: true,
            interval: input.interval,
            failure_threshold: input.failure_threshold,
            recovery_threshold: input.recovery_threshold,
            status: "pending".into(),
            last_check: None,
            latency_ms: None,
            error: None,
            sent: 0,
            received: 0,
            history: vec![],
            tracker: Tracker::default(),
            generation: 0,
        };
        tx.execute(
            "UPDATE targets SET data=? WHERE id=?",
            params![serde_json::to_string(&t).unwrap(), t.id],
        )?;
        tx.commit()?;
        Ok(t)
    }
    pub fn incidents(&self) -> rusqlite::Result<Vec<Incident>> {
        let mut stmt = self.db.prepare("SELECT id,target_name,address,started_at,ended_at,outcome,reason FROM incidents ORDER BY id DESC LIMIT 1000")?;
        let result = stmt
            .query_map([], |r| {
                Ok(Incident {
                    id: r.get(0)?,
                    target_name: r.get(1)?,
                    address: r.get(2)?,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    outcome: r.get(5)?,
                    reason: r.get(6)?,
                })
            })?
            .collect();
        result
    }
    pub fn interrupt(&self, t: &Target, at: i64) -> rusqlite::Result<()> {
        self.db.execute("UPDATE incidents SET ended_at=?,outcome='interrupted' WHERE target_id=? AND ended_at IS NULL", params![at,t.id])?;
        Ok(())
    }
    pub fn reset_after_restart(&self) -> rusqlite::Result<()> {
        for mut t in self.targets()? {
            self.interrupt(&t, t.last_check.unwrap_or_else(now))?;
            t.tracker = Tracker::default();
            t.status = if t.enabled { "pending" } else { "paused" }.into();
            t.latency_ms = None;
            t.error = None;
            self.save(&t)?;
        }
        Ok(())
    }
    pub fn record(
        &mut self,
        id: i64,
        generation: u64,
        result: Result<f64, String>,
        at: i64,
    ) -> rusqlite::Result<()> {
        let mut t = self.target(id)?;
        if !t.enabled || t.generation != generation {
            return Ok(());
        }
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.starts_with("ICMP unavailable:"))
        {
            let tx = self.db.transaction()?;
            tx.execute("UPDATE incidents SET ended_at=?,outcome='interrupted' WHERE target_id=? AND ended_at IS NULL", params![t.last_check.unwrap_or(at), t.id])?;
            t.status = "error".into();
            t.error = result.err();
            t.latency_ms = None;
            t.tracker = Tracker::default();
            tx.execute(
                "UPDATE targets SET data=? WHERE id=?",
                params![serde_json::to_string(&t).unwrap(), t.id],
            )?;
            return tx.commit();
        }
        t.sent += 1;
        t.last_check = Some(at);
        t.latency_ms = result.as_ref().ok().copied();
        t.error = result.as_ref().err().cloned();
        if result.is_ok() {
            t.received += 1;
        }
        t.history.push(t.latency_ms);
        if t.history.len() > 60 {
            t.history.remove(0);
        }
        let transition = t.tracker.record(
            result.is_ok(),
            at,
            t.failure_threshold,
            t.recovery_threshold,
        );
        t.status = if t.tracker.down {
            if result.is_ok() {
                "recovering"
            } else {
                "down"
            }
        } else if result.is_err() {
            "suspect"
        } else {
            "up"
        }
        .into();
        let tx = self.db.transaction()?;
        match transition {
            Some(Transition::Down(start)) => {
                tx.execute("INSERT INTO incidents(target_id,target_name,address,started_at,outcome,reason) VALUES(?,?,?,?,'ongoing',?)",
                    params![t.id,t.name,t.address,start,t.error.as_deref().unwrap_or("No ICMP reply")])?;
            }
            Some(Transition::Recovered(end)) => {
                tx.execute("UPDATE incidents SET ended_at=?,outcome='recovered' WHERE target_id=? AND ended_at IS NULL", params![end,t.id])?;
            }
            None => {}
        }
        tx.execute(
            "UPDATE targets SET data=? WHERE id=?",
            params![serde_json::to_string(&t).unwrap(), t.id],
        )?;
        tx.commit()
    }
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> NewTarget {
        NewTarget {
            name: "Router".into(),
            address: "127.0.0.1".into(),
            interval: 1,
            failure_threshold: 2,
            recovery_threshold: 2,
        }
    }
    #[test]
    fn validates_input() {
        let mut n = input();
        n.address = "127.0.0.1; reboot".into();
        assert!(n.validate().is_err());
        n.address = "::1".into();
        assert!(n.validate().is_ok());
        n.interval = 0;
        assert!(n.validate().is_err());
    }
    #[test]
    fn incident_lifecycle_and_stale_results() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        s.record(t.id, 0, Err("timeout".into()), 1000).unwrap();
        assert!(s.incidents().unwrap().is_empty());
        s.record(t.id, 0, Err("timeout".into()), 2000).unwrap();
        s.record(t.id, 0, Err("timeout".into()), 3000).unwrap();
        assert_eq!(s.incidents().unwrap().len(), 1);
        s.record(t.id, 0, Ok(1.2), 4000).unwrap();
        assert_eq!(s.target(t.id).unwrap().status, "recovering");
        s.record(t.id, 0, Ok(1.2), 5000).unwrap();
        let i = s.incidents().unwrap().remove(0);
        assert_eq!(i.started_at, 1000);
        assert_eq!(i.ended_at, Some(4000));
        assert_eq!(i.outcome, "recovered");
        s.record(t.id, 99, Err("stale".into()), 6000).unwrap();
        assert_eq!(s.target(t.id).unwrap().sent, 5);
    }
    #[test]
    fn removed_ids_are_never_reused_for_new_probe_jobs() {
        let mut s = Store::open(":memory:").unwrap();
        let old = s.add(input()).unwrap();
        s.db.execute("DELETE FROM targets WHERE id=?", [old.id])
            .unwrap();
        let new = s.add(input()).unwrap();
        assert_ne!(old.id, new.id);
    }
    #[test]
    fn socket_errors_do_not_count_as_network_downtime() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        for at in [1000, 2000, 3000] {
            s.record(
                t.id,
                0,
                Err("ICMP unavailable: Operation not permitted".into()),
                at,
            )
            .unwrap();
        }
        assert!(s.incidents().unwrap().is_empty());
        assert_eq!(s.target(t.id).unwrap().status, "error");
        assert_eq!(s.target(t.id).unwrap().sent, 0);
    }
    #[test]
    fn restart_preserves_history_without_inventing_recovery() {
        let path = std::env::temp_dir().join(format!("netcheck-test-{}.db", std::process::id()));
        {
            let mut s = Store::open(path.to_str().unwrap()).unwrap();
            let t = s.add(input()).unwrap();
            s.record(t.id, 0, Err("timeout".into()), 1000).unwrap();
            s.record(t.id, 0, Err("timeout".into()), 2000).unwrap();
        }
        {
            let s = Store::open(path.to_str().unwrap()).unwrap();
            s.reset_after_restart().unwrap();
            let i = s.incidents().unwrap().remove(0);
            assert_eq!(i.outcome, "interrupted");
            assert_eq!(i.ended_at, Some(2000));
            assert_eq!(s.targets().unwrap()[0].sent, 2);
            assert_eq!(s.targets().unwrap()[0].status, "pending");
        }
        std::fs::remove_file(path).unwrap();
    }
}
