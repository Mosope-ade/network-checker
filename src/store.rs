use crate::activity::{self, Event};
use crate::ping::ProbeError;
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
#[serde(deny_unknown_fields)]
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
        let address = self
            .address
            .trim()
            .parse::<IpAddr>()
            .map_err(|_| "Enter a valid IPv4 or IPv6 address (one host per target).".to_string())?
            .to_canonical();
        if address.is_unspecified()
            || address.is_multicast()
            || matches!(address,IpAddr::V4(ip) if ip.is_broadcast())
        {
            return Err("Choose a unicast host address, not an unspecified, multicast, or broadcast address.".into());
        }
        self.address = address.to_string();
        if self.name.is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Name must contain 1–80 characters without control characters.".into());
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
    // Keep the advisory owner lock until the database connection is dropped.
    _owner: Option<std::fs::File>,
}
impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let owner = if path == ":memory:" {
            None
        } else {
            let file = owner_file(std::path::Path::new(path))
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            file.try_lock()
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            Some(file)
        };
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch("PRAGMA journal_mode=WAL;
          CREATE TABLE IF NOT EXISTS targets(id INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS incidents(id INTEGER PRIMARY KEY, target_id INTEGER NOT NULL,
            target_name TEXT NOT NULL, address TEXT NOT NULL, started_at INTEGER NOT NULL,
            ended_at INTEGER, outcome TEXT NOT NULL, reason TEXT NOT NULL);
          CREATE INDEX IF NOT EXISTS incident_target ON incidents(target_id, ended_at);
          CREATE TABLE IF NOT EXISTS activity(id INTEGER PRIMARY KEY AUTOINCREMENT,timestamp INTEGER NOT NULL,target_id INTEGER NOT NULL,target_name TEXT NOT NULL,address TEXT NOT NULL,kind TEXT NOT NULL,message TEXT NOT NULL,outage_started_at INTEGER,duration_ms INTEGER,source TEXT UNIQUE);
          CREATE INDEX IF NOT EXISTS activity_time ON activity(timestamp DESC,id DESC);
          INSERT OR IGNORE INTO activity(timestamp,target_id,target_name,address,kind,message,outage_started_at,source)
          SELECT started_at,target_id,target_name,address,'outage_recorded','Outage retained from earlier downtime history: '||reason,started_at,'incident:'||id||':down' FROM incidents;
          INSERT OR IGNORE INTO activity(timestamp,target_id,target_name,address,kind,message,outage_started_at,duration_ms,source)
          SELECT ended_at,target_id,target_name,address,CASE WHEN outcome='recovered' THEN 'recovery_recorded' ELSE 'interruption_recorded' END,CASE WHEN outcome='recovered' THEN 'Recovery retained from existing downtime history.' ELSE 'Monitoring stopped before recovery was confirmed.' END,started_at,MAX(0,ended_at-started_at),'incident:'||id||':end' FROM incidents WHERE ended_at IS NOT NULL;")?;
        Ok(Self { db, _owner: owner })
    }
    pub fn targets(&self) -> rusqlite::Result<Vec<Target>> {
        let mut stmt = self.db.prepare("SELECT id,data FROM targets ORDER BY id")?;
        let result = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let data: String = r.get(1)?;
                decode_target(&data, id)
            })?
            .collect();
        result
    }
    pub fn target(&self, id: i64) -> rusqlite::Result<Target> {
        let data: String = self
            .db
            .query_row("SELECT data FROM targets WHERE id=?", [id], |r| r.get(0))?;
        decode_target(&data, id)
    }

    #[cfg(test)]
    pub fn save(&self, target: &Target) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE targets SET data=? WHERE id=?",
            params![encode_target(target)?, target.id],
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
            params![encode_target(&t)?, t.id],
        )?;
        activity::write(
            &tx,
            Event::new(
                &t,
                now(),
                "added",
                format!(
                    "Monitoring started. Checking every {} {}. Downtime is confirmed after {} {}.",
                    t.interval,
                    if t.interval == 1 { "second" } else { "seconds" },
                    t.failure_threshold,
                    if t.failure_threshold == 1 {
                        "failed check"
                    } else {
                        "consecutive failed checks"
                    }
                ),
            ),
        )?;
        tx.commit()?;
        Ok(t)
    }
    pub fn incidents(&self) -> rusqlite::Result<Vec<Incident>> {
        let mut stmt = self.db.prepare("SELECT id,target_name,address,started_at,ended_at,outcome,reason FROM incidents ORDER BY ended_at IS NOT NULL, id DESC LIMIT 1000")?;
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
    pub fn reset_after_restart(&mut self) -> rusqlite::Result<()> {
        self.reset_session(true)
    }
    pub fn stop(&mut self) -> rusqlite::Result<()> {
        self.reset_session(false)
    }
    fn reset_session(&mut self, starting: bool) -> rusqlite::Result<()> {
        let targets = self.targets()?;
        let tx = self.db.transaction()?;
        for target in &targets {
            activity::interrupt(
                &tx,
                target,
                target.last_check.unwrap_or_else(now),
                if starting {
                    "The previous monitoring session ended before recovery was confirmed. Unobserved time is excluded."
                } else {
                    "The monitoring service stopped before recovery was confirmed. Unobserved time is excluded."
                },
            )?;
        }
        // Also close any orphaned open incident left by an interrupted older version.
        tx.execute("UPDATE incidents SET ended_at=MAX(started_at,COALESCE((SELECT json_extract(data,'$.last_check') FROM targets WHERE targets.id=incidents.target_id),started_at)),outcome='interrupted' WHERE ended_at IS NULL", [])?;
        for mut t in targets {
            if t.enabled && t.last_check.is_some() {
                activity::write(
                    &tx,
                    Event::new(
                        &t,
                        now(),
                        if starting {
                            "session_started"
                        } else {
                            "session_stopped"
                        },
                        if starting {
                            "Monitoring service started. Saved targets resumed with fresh confirmation checks."
                        } else {
                            "Monitoring service stopped. No further checks run until the service starts again."
                        },
                    ),
                )?;
            }
            t.generation = t.generation.saturating_add(1);
            t.tracker = Tracker::default();
            t.status = if t.enabled { "pending" } else { "paused" }.into();
            t.latency_ms = None;
            t.error = None;
            tx.execute(
                "UPDATE targets SET data=? WHERE id=?",
                params![encode_target(&t)?, t.id],
            )?;
        }
        tx.commit()
    }
    pub fn set_enabled(&mut self, id: i64, enabled: bool) -> rusqlite::Result<()> {
        let mut t = self.target(id)?;
        if t.enabled == enabled {
            return Ok(());
        }
        let tx = self.db.transaction()?;
        activity::interrupt(
            &tx,
            &t,
            t.last_check.unwrap_or_else(now),
            "Monitoring was paused before recovery was confirmed.",
        )?;
        activity::write(
            &tx,
            Event::new(
                &t,
                now(),
                if enabled { "resumed" } else { "paused" },
                if enabled {
                    "Monitoring resumed. Waiting for a fresh ping result."
                } else {
                    "Monitoring paused by the user. No more probes will be sent."
                },
            ),
        )?;
        t.enabled = enabled;
        t.generation = t.generation.saturating_add(1);
        t.tracker = Tracker::default();
        t.status = if enabled { "pending" } else { "paused" }.into();
        t.latency_ms = None;
        t.error = None;
        tx.execute(
            "UPDATE targets SET data=? WHERE id=?",
            params![encode_target(&t)?, id],
        )?;
        tx.commit()
    }
    pub fn record(
        &mut self,
        id: i64,
        generation: u64,
        result: Result<f64, ProbeError>,
        at: i64,
    ) -> rusqlite::Result<()> {
        let mut t = self.target(id)?;
        if !t.enabled || t.generation != generation {
            return Ok(());
        }
        if let Err(error) = &result {
            tracing::debug!(target=id,details=%error,"ICMP probe failed");
        }
        let tx = self.db.transaction()?;
        if t.status != "pending"
            && t.last_check.is_some_and(|previous| {
                at < previous || at.saturating_sub(previous) > (t.interval as i64 + 7) * 1000
            })
        {
            activity::interrupt(&tx,&t,t.last_check.unwrap_or(at),"A monitoring gap or clock change interrupted this outage. Unobserved time is excluded.")?;
            let previous = t.last_check.unwrap_or(at);
            let (kind, message) = if at < previous {
                ("clock_changed","The system clock moved backward. The previous outage was interrupted and confirmation checks restarted.".to_owned())
            } else {
                ("gap",format!("No probe result was recorded for {} seconds. Monitoring has resumed with fresh confirmation checks.",at.saturating_sub(previous)/1000))
            };
            activity::write(&tx, Event::new(&t, at, kind, message))?;
            t.tracker = Tracker::default();
        }
        if matches!(&result, Err(ProbeError::Unavailable(_))) {
            activity::interrupt(
                &tx,
                &t,
                t.last_check.unwrap_or(at),
                "Local ping permissions or socket failure interrupted monitoring.",
            )?;
            if t.status != "error" {
                activity::write(&tx,Event::new(&t,at,"probe_error","Local ping probes could not run. Check this machine's ICMP permissions; this is not confirmed network downtime."))?;
            }
            t.status = "error".into();
            t.error = result.err().map(|error| error.user_message().to_owned());
            t.latency_ms = None;
            t.tracker = Tracker::default();
            tx.execute(
                "UPDATE targets SET data=? WHERE id=?",
                params![encode_target(&t)?, t.id],
            )?;
            return tx.commit();
        }
        if result
            .as_ref()
            .is_ok_and(|value| !value.is_finite() || *value < 0.)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Invalid probe latency".into(),
            ));
        }
        let previous_status = t.status.clone();
        t.sent = t.sent.saturating_add(1);
        t.last_check = Some(at);
        t.latency_ms = result.as_ref().ok().copied();
        t.error = result
            .as_ref()
            .err()
            .map(|error| error.user_message().to_owned());
        if result.is_ok() {
            t.received = t.received.saturating_add(1);
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
        if previous_status == "pending" && result.is_ok() {
            activity::write(
                &tx,
                Event::new(
                    &t,
                    at,
                    "responding",
                    format!(
                        "First reply received in {:.2} ms.",
                        t.latency_ms.unwrap_or_default()
                    ),
                ),
            )?;
        } else if previous_status == "error" && result.is_ok() {
            activity::write(
                &tx,
                Event::new(
                    &t,
                    at,
                    "responding",
                    "Local probes are working again and the target is responding.",
                ),
            )?;
        } else if previous_status == "suspect" && result.is_ok() && !t.tracker.down {
            activity::write(&tx,Event::new(&t,at,"transient_recovery","Replies resumed before the downtime threshold was reached. No outage was confirmed."))?;
        }
        if t.status == "suspect" && t.tracker.failures == 1 {
            activity::write(&tx,Event::new(&t,at,"ping_failed",format!("Ping did not receive a reply. Waiting for {} consecutive failed checks before confirming downtime.",t.failure_threshold)))?;
        }
        if t.status == "recovering" && t.tracker.successes == 1 {
            activity::write(&tx,Event::new(&t,at,"recovering",format!("Reply received. Waiting for {} consecutive successful checks to confirm recovery.",t.recovery_threshold)))?;
        }
        match transition {
            Some(Transition::Down(start)) => {
                tx.execute("INSERT INTO incidents(target_id,target_name,address,started_at,outcome,reason) VALUES(?,?,?,?,'ongoing',?)",
                    params![t.id,t.name,t.address,start,t.error.as_deref().unwrap_or("No ICMP reply")])?;
                let incident = tx.last_insert_rowid();
                activity::write(
                    &tx,
                    Event::new(
                        &t,
                        at,
                        "outage_confirmed",
                        format!(
                            "{} consecutive checks failed. Downtime is now confirmed.",
                            t.failure_threshold
                        ),
                    )
                    .outage(start, None)
                    .source(format!("incident:{incident}:down")),
                )?;
            }
            Some(Transition::Recovered(end)) => {
                let (incident,start):(i64,i64)=tx.query_row("SELECT id,started_at FROM incidents WHERE target_id=? AND ended_at IS NULL ORDER BY id DESC LIMIT 1",[t.id],|row|Ok((row.get(0)?,row.get(1)?)))?;
                activity::write(&tx,Event::new(&t,at,"recovered",format!("Recovery confirmed after {} {}. Downtime ends at the first reply in this stable sequence.",t.recovery_threshold,if t.recovery_threshold==1 {"successful reply"} else {"consecutive successful replies"})).outage(start,Some(end.saturating_sub(start))).source(format!("incident:{incident}:end")))?;
                tx.execute("UPDATE incidents SET ended_at=?,outcome='recovered' WHERE target_id=? AND ended_at IS NULL", params![end,t.id])?;
            }
            None => {}
        }
        tx.execute(
            "UPDATE targets SET data=? WHERE id=?",
            params![encode_target(&t)?, t.id],
        )?;
        tx.commit()
    }
}
// Windows file locks deny normal reads/writes on other handles, including SQLite's.
// Lock a canonical-path sidecar there; Unix keeps its advisory inode lock on the DB.
fn owner_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        drop(options.open(path)?);
        let mut lock_path = std::fs::canonicalize(path)?.into_os_string();
        lock_path.push(".owner.lock");
        options.open(lock_path)
    }
    #[cfg(not(windows))]
    {
        options.open(path)
    }
}
fn encode_target(t: &Target) -> rusqlite::Result<String> {
    serde_json::to_string(t).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}
fn decode_target(data: &str, id: i64) -> rusqlite::Result<Target> {
    let invalid = |error: Box<dyn std::error::Error + Send + Sync>| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error)
    };
    let target: Target = serde_json::from_str(data).map_err(|e| invalid(Box::new(e)))?;
    let mut input = NewTarget {
        name: target.name.clone(),
        address: target.address.clone(),
        interval: target.interval,
        failure_threshold: target.failure_threshold,
        recovery_threshold: target.recovery_threshold,
    };
    input
        .validate()
        .map_err(|e| invalid(Box::new(std::io::Error::other(e))))?;
    if target.id != id
        || target.received > target.sent
        || target.history.len() > 60
        || ![
            "pending",
            "paused",
            "up",
            "suspect",
            "down",
            "recovering",
            "error",
        ]
        .contains(&target.status.as_str())
        || target
            .history
            .iter()
            .chain(std::iter::once(&target.latency_ms))
            .flatten()
            .any(|v| !v.is_finite() || *v < 0.)
    {
        return Err(invalid(Box::new(std::io::Error::other(
            "Invalid persisted target state",
        ))));
    }
    Ok(target)
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
            address: std::net::Ipv4Addr::LOCALHOST.to_string(),
            interval: 1,
            failure_threshold: 2,
            recovery_threshold: 2,
        }
    }
    #[test]
    fn monitoring_gap_does_not_become_continuous_downtime() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 1000)
            .unwrap();
        s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 2000)
            .unwrap();
        s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 100000)
            .unwrap();
        let incidents = s.incidents().unwrap();
        assert_eq!(incidents[0].outcome, "interrupted");
        assert_eq!(incidents[0].ended_at, Some(2000));
        assert_eq!(s.target(t.id).unwrap().status, "suspect");
    }
    #[test]
    fn corrupt_persisted_targets_are_rejected_before_starting_a_probe_task() {
        let mut s = Store::open(":memory:").unwrap();
        let mut t = s.add(input()).unwrap();
        t.interval = 0;
        s.save(&t).unwrap();
        assert!(s.targets().is_err());
    }
    #[test]
    fn database_has_a_single_monitor_owner() {
        let path = std::env::temp_dir().join(format!("netcheck-owner-{}.db", std::process::id()));
        let first = Store::open(path.to_str().unwrap()).unwrap();
        assert!(Store::open(path.to_str().unwrap()).is_err());
        drop(first);
        drop(Store::open(path.to_str().unwrap()).unwrap());
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
    }
    #[test]
    fn explicit_state_changes_are_idempotent_and_discard_old_results() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        s.set_enabled(t.id, false).unwrap();
        s.set_enabled(t.id, false).unwrap();
        assert_eq!(s.target(t.id).unwrap().generation, 1);
        s.set_enabled(t.id, true).unwrap();
        s.record(t.id, 0, Ok(1.), 1000).unwrap();
        assert_eq!(s.target(t.id).unwrap().sent, 0);
        s.reset_after_restart().unwrap();
        s.record(t.id, 2, Ok(1.), 2000).unwrap();
        assert_eq!(s.target(t.id).unwrap().sent, 0);
    }
    #[test]
    fn rejects_non_host_addresses_and_normalizes_equivalent_addresses() {
        let mut n = input();
        n.address = std::net::Ipv4Addr::UNSPECIFIED.to_string();
        assert!(n.validate().is_err());
        n.address = std::net::Ipv4Addr::BROADCAST.to_string();
        assert!(n.validate().is_err());
        n.address = std::net::Ipv4Addr::LOCALHOST.to_ipv6_mapped().to_string();
        n.validate().unwrap();
        assert_eq!(n.address, std::net::Ipv4Addr::LOCALHOST.to_string());
    }
    #[test]
    fn invalid_latency_cannot_corrupt_saved_state() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        assert!(s.record(t.id, 0, Ok(f64::NAN), 1000).is_err());
        assert_eq!(s.target(t.id).unwrap().sent, 0);
    }
    #[test]
    fn activity_distinguishes_failure_confirmation_and_stable_recovery_times() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        s.record(t.id, 0, Err(ProbeError::TimedOut), 1000).unwrap();
        s.record(t.id, 0, Err(ProbeError::TimedOut), 2000).unwrap();
        s.record(t.id, 0, Ok(1.), 3000).unwrap();
        s.record(t.id, 0, Ok(1.), 4000).unwrap();
        let events = activity::list(&s.db).unwrap();
        let failure = events
            .iter()
            .find(|event| event.kind == "outage_confirmed")
            .unwrap();
        assert_eq!(failure.timestamp, 2000);
        assert_eq!(failure.outage_started_at, Some(1000));
        let recovery = events
            .iter()
            .find(|event| event.kind == "recovered")
            .unwrap();
        assert_eq!(recovery.timestamp, 4000);
        assert_eq!(recovery.duration_ms, Some(2000));
        assert!(events
            .iter()
            .any(|event| event.kind == "ping_failed" && event.timestamp == 1000));
        assert!(events
            .iter()
            .any(|event| event.kind == "recovering" && event.timestamp == 3000));
        s.stop().unwrap();
        assert!(activity::list(&s.db)
            .unwrap()
            .iter()
            .any(|event| event.kind == "session_stopped"));
    }
    #[test]
    fn existing_downtime_is_migrated_to_activity_once_without_inventing_confirmation_times() {
        let path = std::env::temp_dir().join(format!(
            "netcheck-activity-{}-{}.db",
            std::process::id(),
            now()
        ));
        {
            let s = Store::open(path.to_str().unwrap()).unwrap();
            s.db.execute_batch("DROP TABLE activity;").unwrap();
            s.db.execute("INSERT INTO incidents(target_id,target_name,address,started_at,ended_at,outcome,reason) VALUES(1,'Historical router',?,1000,2000,'recovered','No reply')",[std::net::Ipv4Addr::LOCALHOST.to_string()]).unwrap();
        }
        for _ in 0..2 {
            let s = Store::open(path.to_str().unwrap()).unwrap();
            let events = activity::list(&s.db).unwrap();
            assert_eq!(events.len(), 2);
            assert!(events
                .iter()
                .any(|event| event.kind == "outage_recorded" && event.timestamp == 1000));
            assert!(events
                .iter()
                .any(|event| event.kind == "recovery_recorded" && event.timestamp == 2000));
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn validates_input() {
        let mut n = input();
        n.address = "invalid-address; command".into();
        assert!(n.validate().is_err());
        n.address = std::net::Ipv6Addr::LOCALHOST.to_string();
        assert!(n.validate().is_ok());
        n.interval = 0;
        assert!(n.validate().is_err());
    }
    #[test]
    fn incident_lifecycle_and_stale_results() {
        let mut s = Store::open(":memory:").unwrap();
        let t = s.add(input()).unwrap();
        s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 1000)
            .unwrap();
        assert!(s.incidents().unwrap().is_empty());
        s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 2000)
            .unwrap();
        s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 3000)
            .unwrap();
        assert_eq!(s.incidents().unwrap().len(), 1);
        s.record(t.id, 0, Ok(1.2), 4000).unwrap();
        assert_eq!(s.target(t.id).unwrap().status, "recovering");
        s.record(t.id, 0, Ok(1.2), 5000).unwrap();
        let i = s.incidents().unwrap().remove(0);
        assert_eq!(i.started_at, 1000);
        assert_eq!(i.ended_at, Some(4000));
        assert_eq!(i.outcome, "recovered");
        s.record(t.id, 99, Err(ProbeError::NoReply("stale".into())), 6000)
            .unwrap();
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
                Err(ProbeError::Unavailable("Operation not permitted".into())),
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
            s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 1000)
                .unwrap();
            s.record(t.id, 0, Err(ProbeError::NoReply("timeout".into())), 2000)
                .unwrap();
        }
        {
            let mut s = Store::open(path.to_str().unwrap()).unwrap();
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
