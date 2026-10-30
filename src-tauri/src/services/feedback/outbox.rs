//! Bounded cumulative journal. One atomic replacement precedes every in-memory commit.
use crate::{models::feedback::*, utils::atomic_file::write_atomically};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const MAX_SESSIONS: usize = 100;
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_AGE_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Journal {
    pub schema_version: u32,
    pub installation_id: String,
    pub sessions: BTreeMap<String, Rental>,
    pub dropped_sessions: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rental {
    pub id: String,
    pub created_at: i64,
    pub confirmed: bool,
    #[serde(default)]
    pub creation_reconciled: bool,
    pub snapshot: SessionSnapshot,
    pub accepted_revision: u64,
    pub review: Option<ReviewInput>,
    // High-water mark survives a cleared draft; default migrates older journals.
    #[serde(default)]
    pub review_revision: u64,
    pub accepted_review_revision: u64,
    pub blocked: bool,
    pub review_blocked: bool,
}
#[derive(Clone, Copy)]
pub enum Milestone {
    Ready,
    Paired,
    FirstFrame,
    Destroyed,
}

impl Rental {
    pub fn pending(
        epoch: u64,
        machine: Option<String>,
        offer: String,
        hardware: HardwareSnapshot,
    ) -> Self {
        let at = chrono::Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().timestamp(),
            confirmed: false,
            creation_reconciled: false,
            accepted_revision: 0,
            review: None,
            review_revision: 0,
            accepted_review_revision: 0,
            blocked: false,
            review_blocked: false,
            snapshot: SessionSnapshot {
                schema_version: 1,
                revision: 1,
                consent_epoch: epoch,
                provider: "vast".into(),
                provider_machine_id: machine.unwrap_or_default(),
                provider_instance_id: String::new(),
                provider_offer_id: Some(offer),
                lifecycle: Lifecycle {
                    rental_started_at: at,
                    ready_at: None,
                    paired_at: None,
                    setup_success_at: None,
                    first_stream_frame_at: None,
                    destroyed_at: None,
                    observation_complete: true,
                    last_event_sequence: 1,
                    event_order: EventOrder {
                        rental_started: 1,
                        ..Default::default()
                    },
                },
                context: SessionContext {
                    platform: std::env::consts::OS.into(),
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    collector_version: "android_lifecycle_frames_v1_no_jitter".into(),
                },
                hardware_snapshot: hardware,
                jitter_buckets: Vec::new(),
            },
        }
    }
    pub fn confirm(&mut self, instance: String, complete: bool) {
        if self.confirmed {
            return;
        }
        self.confirmed = true;
        self.snapshot.provider_instance_id = instance;
        self.snapshot.lifecycle.rental_started_at = chrono::Utc::now().to_rfc3339();
        self.snapshot.lifecycle.observation_complete &= complete;
        self.snapshot.revision += 1;
    }
    pub fn mark(&mut self, event: Milestone) -> bool {
        if !self.confirmed || self.snapshot.lifecycle.destroyed_at.is_some() {
            return false;
        }
        let life = &mut self.snapshot.lifecycle;
        let (time, sequence) = match event {
            Milestone::Ready => (&mut life.ready_at, &mut life.event_order.ready),
            Milestone::Paired => (&mut life.paired_at, &mut life.event_order.paired),
            Milestone::FirstFrame => (
                &mut life.first_stream_frame_at,
                &mut life.event_order.first_stream_frame,
            ),
            Milestone::Destroyed => (&mut life.destroyed_at, &mut life.event_order.destroyed),
        };
        if time.is_some() {
            return false;
        }
        life.last_event_sequence += 1;
        *time = Some(chrono::Utc::now().to_rfc3339());
        *sequence = Some(life.last_event_sequence);
        if life.ready_at.is_some()
            && life.paired_at.is_some()
            && life.setup_success_at.is_none()
            && life.destroyed_at.is_none()
        {
            life.last_event_sequence += 1;
            life.setup_success_at = Some(chrono::Utc::now().to_rfc3339());
            life.event_order.setup_success = Some(life.last_event_sequence);
        }
        self.snapshot.revision += 1;
        true
    }
    pub fn gap(&mut self) -> bool {
        if self.confirmed
            && self.snapshot.lifecycle.destroyed_at.is_none()
            && self.snapshot.lifecycle.observation_complete
        {
            self.snapshot.lifecycle.observation_complete = false;
            self.snapshot.revision += 1;
            true
        } else {
            false
        }
    }
    pub fn review_eligible(&self) -> bool {
        self.confirmed
            && self.snapshot.lifecycle.first_stream_frame_at.is_some()
            && self.snapshot.lifecycle.destroyed_at.is_none()
            && !self.blocked
    }
    fn valid(&self) -> bool {
        let s = &self.snapshot;
        let life = &s.lifecycle;
        let order = &life.event_order;
        if uuid::Uuid::parse_str(&self.id).is_err()
            || s.schema_version != 1
            || s.revision == 0
            || s.revision > 9_007_199_254_740_991
            || self.accepted_revision > s.revision
            || s.provider != "vast"
            || s.consent_epoch == 0
            || s.consent_epoch > 9_007_199_254_740_991
            || (!s.provider_machine_id.is_empty() && !valid_provider_id(&s.provider_machine_id))
            || (self.confirmed && !valid_provider_id(&s.provider_instance_id))
            || s.provider_offer_id
                .as_ref()
                .is_some_and(|s| !valid_provider_id(s))
            || order.rental_started != 1
            || life.last_event_sequence < 1
            || life.last_event_sequence > 9_007_199_254_740_991
            || chrono::DateTime::parse_from_rfc3339(&life.rental_started_at).is_err()
            || !s.jitter_buckets.is_empty()
        {
            return false;
        }
        let mut sequences = std::collections::BTreeSet::from([1]);
        for (at, seq) in [
            (&life.ready_at, order.ready),
            (&life.paired_at, order.paired),
            (&life.setup_success_at, order.setup_success),
            (&life.first_stream_frame_at, order.first_stream_frame),
            (&life.destroyed_at, order.destroyed),
        ] {
            match (at, seq) {
                (None, None) => {}
                (Some(at), Some(seq))
                    if chrono::DateTime::parse_from_rfc3339(at).is_ok()
                        && seq > 1
                        && seq <= life.last_event_sequence
                        && sequences.insert(seq) => {}
                _ => return false,
            }
        }
        if let Some(success) = order.setup_success {
            if order.ready.is_none_or(|n| n >= success) || order.paired.is_none_or(|n| n >= success)
            {
                return false;
            }
        }
        if let Some(destroyed) = order.destroyed {
            if [
                order.ready,
                order.paired,
                order.setup_success,
                order.first_stream_frame,
            ]
            .into_iter()
            .flatten()
            .any(|n| n >= destroyed)
            {
                return false;
            }
        }
        if let Some(review) = &self.review {
            if order.first_stream_frame.is_none()
                || review.revision == 0
                || self.accepted_review_revision > review.revision
                || ReviewInput::new(review.revision, review.score, review.comment.clone()).is_err()
            {
                return false;
            }
        } else if self.accepted_review_revision != 0 {
            return false;
        }
        true
    }
    pub fn deliverable(&self, epoch: u64) -> bool {
        self.confirmed
            && !self.blocked
            && self.snapshot.consent_epoch == epoch
            && valid_provider_id(&self.snapshot.provider_machine_id)
            && valid_provider_id(&self.snapshot.provider_instance_id)
            && self.snapshot.jitter_buckets.is_empty()
    }
}
impl Journal {
    pub fn empty(installation_id: String) -> Self {
        Self {
            schema_version: 1,
            installation_id,
            sessions: BTreeMap::new(),
            dropped_sessions: 0,
        }
    }
    pub fn rental_mut(&mut self, instance: u64) -> Option<&mut Rental> {
        let id = instance.to_string();
        self.sessions
            .values_mut()
            .find(|r| r.confirmed && r.snapshot.provider_instance_id == id)
    }
    pub fn rental(&self, instance: &str) -> Option<&Rental> {
        self.sessions
            .values()
            .find(|r| r.confirmed && r.snapshot.provider_instance_id == instance)
    }
    pub fn rebase_epoch(&mut self, epoch: u64) {
        for r in self.sessions.values_mut() {
            if r.snapshot.consent_epoch != epoch {
                r.snapshot.consent_epoch = epoch;
                r.snapshot.revision += 1;
            }
        }
    }
    pub fn prune(&mut self, now: i64) {
        self.sessions.retain(|_, r| {
            let keep = now.saturating_sub(r.created_at) <= MAX_AGE_SECONDS;
            if !keep {
                self.dropped_sessions = self.dropped_sessions.saturating_add(1);
            }
            keep
        });
        while self.sessions.len() > MAX_SESSIONS
            || serde_json::to_vec(self).map_or(true, |b| b.len() > MAX_BYTES)
        {
            // Completed/acknowledged routine history is discarded before pending lifecycles/reviews.
            let id = self
                .sessions
                .values()
                .min_by_key(|r| {
                    (
                        !(r.snapshot.lifecycle.destroyed_at.is_some()
                            && r.accepted_revision == r.snapshot.revision
                            && r.review
                                .as_ref()
                                .is_none_or(|v| v.revision == r.accepted_review_revision)),
                        r.created_at,
                    )
                })
                .map(|r| r.id.clone());
            let Some(id) = id else {
                break;
            };
            self.sessions.remove(&id);
            self.dropped_sessions = self.dropped_sessions.saturating_add(1);
        }
    }
}

pub struct Outbox {
    path: PathBuf,
}
impl Outbox {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("feedback-v1/outbox.json"),
        }
    }
    pub fn load(&self, installation: &str) -> Result<Journal, String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Journal::empty(installation.into()))
            }
            Err(_) => return Err("Feedback journal could not be read; collection is paused".into()),
        };
        if bytes.len() > MAX_BYTES {
            return Err("Feedback journal exceeds its storage limit".into());
        }
        let mut journal: Journal = serde_json::from_slice(&bytes)
            .map_err(|_| "Feedback journal is invalid; collection is paused")?;
        if journal.schema_version != 1
            || journal.installation_id != installation
            || journal.sessions.len() > MAX_SESSIONS
            || journal
                .sessions
                .iter()
                .any(|(id, r)| id != &r.id || !r.valid())
        {
            return Err("Feedback journal version or ownership cannot be verified".into());
        }
        for rental in journal.sessions.values_mut() {
            rental.gap();
        }
        journal.prune(chrono::Utc::now().timestamp());
        Ok(journal)
    }
    pub fn save(&self, journal: &Journal) -> Result<(), String> {
        let bytes = serde_json::to_vec(journal).map_err(|_| "Could not encode feedback journal")?;
        if bytes.len() > MAX_BYTES || journal.sessions.len() > MAX_SESSIONS {
            return Err("Feedback storage limit reached".into());
        }
        write_atomically(&self.path, &bytes)
            .map_err(|_| "Could not persist feedback journal".into())
    }
    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                #[cfg(unix)]
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::File::open(parent).and_then(|f| f.sync_all());
                }
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not clear unsent feedback; delivery remains disabled".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_is_idempotent_and_recovery_never_fabricates_failure() {
        let mut r = Rental::pending(
            1,
            Some("12".into()),
            "13".into(),
            HardwareSnapshot::default(),
        );
        assert!(!r.mark(Milestone::Ready));
        r.confirm("14".into(), true);
        r.mark(Milestone::Paired);
        assert!(r.snapshot.lifecycle.setup_success_at.is_none());
        r.gap();
        assert!(!r.snapshot.lifecycle.observation_complete);
        r.mark(Milestone::Ready);
        let revision = r.snapshot.revision;
        assert!(r.snapshot.lifecycle.setup_success_at.is_some());
        assert!(!r.mark(Milestone::Ready));
        assert_eq!(revision, r.snapshot.revision);
        assert!(!r.review_eligible());
        r.mark(Milestone::FirstFrame);
        assert!(r.review_eligible());
        r.mark(Milestone::Destroyed);
        assert!(r.snapshot.lifecycle.setup_success_at.is_some());
        assert!(!r.mark(Milestone::Ready));
    }
    #[test]
    fn atomic_roundtrip_has_one_rental_and_independent_review_revision() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let outbox = Outbox::new(&root);
        let mut j = Journal::empty("installation".into());
        let mut r = Rental::pending(1, Some("1".into()), "2".into(), HardwareSnapshot::default());
        r.confirm("3".into(), true);
        r.mark(Milestone::FirstFrame);
        r.review = Some(ReviewInput::new(1, Some(8), None).unwrap());
        let id = r.id.clone();
        j.sessions.insert(id.clone(), r);
        outbox.save(&j).unwrap();
        let restored = outbox.load("installation").unwrap();
        assert_eq!(restored.sessions.len(), 1);
        assert_eq!(restored.sessions[&id].review.as_ref().unwrap().revision, 1);
        assert!(
            !restored.sessions[&id]
                .snapshot
                .lifecycle
                .observation_complete
        );
        assert!(outbox.load("another installation").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn scope_replay_preserves_rental_identity_and_independent_review() {
        let mut journal = Journal::empty("installation".into());
        let mut rental =
            Rental::pending(1, Some("4".into()), "5".into(), HardwareSnapshot::default());
        rental.confirm("6".into(), true);
        rental.mark(Milestone::Ready);
        rental.mark(Milestone::Paired);
        rental.mark(Milestone::FirstFrame);
        rental.review = Some(ReviewInput::new(2, None, Some("smooth".into())).unwrap());
        let id = rental.id.clone();
        journal.sessions.insert(id.clone(), rental);
        journal.rebase_epoch(2);
        let revision = journal.sessions[&id].snapshot.revision;
        journal.rebase_epoch(2);
        let rental = &journal.sessions[&id];
        assert_eq!(rental.snapshot.revision, revision);
        assert_eq!(rental.review.as_ref().unwrap().revision, 2);
        assert_eq!(rental.snapshot.provider_instance_id, "6");
        assert!(rental.valid());
    }
    #[test]
    fn corruption_and_future_payloads_are_rejected_not_repaired() {
        let mut rental =
            Rental::pending(1, Some("4".into()), "5".into(), HardwareSnapshot::default());
        rental.confirm("6".into(), true);
        assert!(rental.valid());
        rental.snapshot.lifecycle.event_order.ready = Some(2);
        assert!(!rental.valid());
        rental.snapshot.lifecycle.event_order.ready = None;
        rental.snapshot.schema_version = 99;
        assert!(!rental.valid());
    }
    #[test]
    fn pending_data_expires_without_creating_a_terminal_outcome() {
        let mut journal = Journal::empty("installation".into());
        let rental = Rental::pending(1, Some("4".into()), "5".into(), HardwareSnapshot::default());
        let expired_at = rental.created_at + MAX_AGE_SECONDS + 1;
        journal.sessions.insert(rental.id.clone(), rental);
        journal.prune(expired_at);
        assert!(journal.sessions.is_empty());
        assert_eq!(journal.dropped_sessions, 1);
    }
    #[test]
    fn destruction_is_not_produced_by_checkpoint_gap_or_review_staging() {
        let mut rental =
            Rental::pending(1, Some("4".into()), "5".into(), HardwareSnapshot::default());
        rental.confirm("6".into(), true);
        rental.gap();
        rental.mark(Milestone::FirstFrame);
        rental.review = Some(ReviewInput::new(1, Some(7), None).unwrap());
        assert!(rental.snapshot.lifecycle.destroyed_at.is_none());
        assert!(!rental.snapshot.lifecycle.observation_complete);
        assert!(rental.snapshot.lifecycle.setup_success_at.is_none());
        rental.mark(Milestone::Destroyed);
        let sequence = rental.snapshot.lifecycle.last_event_sequence;
        assert!(!rental.mark(Milestone::Destroyed));
        assert_eq!(rental.snapshot.lifecycle.last_event_sequence, sequence);
    }
    #[test]
    fn bounded_queue_never_invents_terminal_evidence() {
        let mut j = Journal::empty("i".into());
        for _ in 0..105 {
            let mut r = Rental::pending(1, None, "2".into(), HardwareSnapshot::default());
            r.confirm("3".into(), true);
            j.sessions.insert(r.id.clone(), r);
        }
        j.prune(chrono::Utc::now().timestamp());
        assert_eq!(j.sessions.len(), MAX_SESSIONS);
        assert_eq!(j.dropped_sessions, 5);
        assert!(j
            .sessions
            .values()
            .all(|r| r.snapshot.lifecycle.destroyed_at.is_none()));
    }
}
