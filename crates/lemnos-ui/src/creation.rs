// Creating a machine, after its slot was reserved (see
// `Database::reserve_machine`): copying the template in the background, and
// picking up copies whose outcome Lemnos didn't see.
//
//   pending ──copy finished, tagged──> active
//      │
//      └──copy failed, or lost track──> failed (frees the slot)
//
// The page that asked for the machine doesn't wait: it shows the record,
// which says "Being created" until this is done. A copy still running when
// Lemnos stops waiting (or stops altogether) stays pending; `sweep` runs
// `reconcile` on it every minute, which finishes the job from the copy's
// task (its UPID), not from the guest's tags: those are only set once the
// copy is done.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lemnos_db::{Database, MachineRecord, MachineStatus, ProxmoxEnvironment};
use lemnos_proxmox::{
    client::{Client, Failure, ReqwestError},
    guests::{self, GuestId, VmId, provision},
    tasks::{self, Progress, Task},
};

use crate::app::console::environments;

/// How many IDs a copy tries before giving up. Each one is taken only when
/// another request was handed the same free ID at the same moment.
pub const ATTEMPTS: usize = 3;
/// How long the background job follows a copy. A copy can take longer; it
/// then stays pending until `reconcile` sees it done.
const COPY_WAIT: Duration = Duration::from_secs(30 * 60);
/// A pending record with no copy Proxmox knows of, this old, isn't going to
/// get one: Lemnos stopped before starting it, or lost its task.
pub const STALE_AFTER: Duration = Duration::from_secs(15 * 60);
/// A pending machine's page looks at Proxmox itself once it is this old.
pub const LOOK_AFTER: Duration = Duration::from_secs(60);
/// How often `sweep` looks at pending machines.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Why a creation failed: in words for the owner, and the error behind it
/// for admins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub reason: String,
    pub detail: String,
}

impl Failed {
    fn new(reason: impl Into<String>, detail: impl std::fmt::Display) -> Self {
        Self {
            reason: reason.into(),
            detail: detail.to_string(),
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

pub fn age(record: &MachineRecord) -> Duration {
    Duration::from_secs(now().saturating_sub(record.created_at))
}

/// What starting a copy takes, so the retries can be tested without a
/// Proxmox.
pub trait CopySteps {
    /// A free VM ID.
    fn next_id(&self) -> impl Future<Output = Result<VmId, Failed>> + Send;
    /// Notes the ID on the record before the copy is asked for, and the
    /// task once it started.
    fn note(
        &self,
        vmid: VmId,
        task: Option<&Task>,
    ) -> impl Future<Output = Result<(), Failed>> + Send;
    /// Asks Proxmox for the copy.
    fn copy(&self, vmid: VmId) -> impl Future<Output = Result<Task, ReqwestError>> + Send;
}

/// Starts the copy. When the ID turns out taken (another request got the
/// same one), it takes a new one and tries again, up to `attempts` times.
pub async fn start_copy(steps: &impl CopySteps, attempts: usize) -> Result<(VmId, Task), Failed> {
    let mut last = None;
    for _ in 0..attempts {
        let vmid = steps.next_id().await?;
        steps.note(vmid, None).await?;
        match steps.copy(vmid).await {
            Ok(task) => {
                steps.note(vmid, Some(&task)).await?;
                return Ok((vmid, task));
            }
            Err(error) if provision::id_taken(&error) => last = Some(error),
            Err(error) => {
                return Err(Failed::new("Proxmox refused to copy the template.", error));
            }
        }
    }
    Err(Failed::new(
        "Proxmox kept giving out IDs that were already taken. Try again in a moment.",
        last.map_or_else(String::new, |error| error.to_string()),
    ))
}

struct Steps<'a> {
    database: &'a Database,
    client: &'a Client,
    record: &'a MachineRecord,
    template: &'a GuestId,
}

impl CopySteps for Steps<'_> {
    async fn next_id(&self) -> Result<VmId, Failed> {
        provision::next_free_id(self.client)
            .await
            .map_err(|error| Failed::new("Proxmox didn't hand out an ID for the machine.", error))
    }

    async fn note(&self, vmid: VmId, task: Option<&Task>) -> Result<(), Failed> {
        self.database
            .machine_started(
                self.record.environment,
                &self.record.machine,
                &vmid.to_string(),
                task.map(|task| task.upid.as_str()),
            )
            .await
            .map_err(|error| Failed::new("Lemnos couldn't save the machine's progress.", error))
    }

    async fn copy(&self, vmid: VmId) -> Result<Task, ReqwestError> {
        provision::start_copy(
            self.client,
            self.template,
            vmid,
            &self.record.name,
            self.record.description.as_deref(),
        )
        .await
    }
}

/// Copies `template` for the pending `record`, in the background.
pub fn spawn(
    database: Database,
    environment: ProxmoxEnvironment,
    template: GuestId,
    record: MachineRecord,
) {
    // Taken before the job starts, so a sweep can't get in first.
    let claim = claim(&record);
    tokio::spawn(async move {
        let _claim = claim;
        if let Err(failed) = create(&database, &environment, &template, &record).await {
            fail(&database, &record, &failed).await;
        }
    });
}

async fn create(
    database: &Database,
    environment: &ProxmoxEnvironment,
    template: &GuestId,
    record: &MachineRecord,
) -> Result<(), Failed> {
    let client = environments::connect(environment).await.map_err(|error| {
        Failed::new(
            format!(
                "Lemnos couldn't reach {} to create the machine.",
                environment.name
            ),
            error,
        )
    })?;
    let steps = Steps {
        database,
        client: &client,
        record,
        template,
    };
    let (vmid, task) = start_copy(&steps, ATTEMPTS).await?;
    match tasks::wait(&client, &task, COPY_WAIT).await {
        Ok(()) => {}
        Err(tasks::Error::Failed(status)) => {
            return Err(Failed::new("Proxmox couldn't copy the template.", status));
        }
        // Still copying, or Proxmox didn't answer: the record stays pending
        // and `reconcile` takes it from here.
        Err(error) => {
            eprintln!("machine `{}`: left pending: {error}", record.name);
            return Ok(());
        }
    }
    // The copy is made on the template's node.
    let guest = GuestId {
        node: template.node.clone(),
        vmid,
        kind: template.kind,
    };
    finish(database, &client, record, &guest).await;
    Ok(())
}

/// Gives the copied guest its tags and marks the record active. If the tags
/// can't be written, the record stays pending and `reconcile` tries again.
async fn finish(database: &Database, client: &Client, record: &MachineRecord, guest: &GuestId) {
    if let Err(error) = provision::tag_new_copy(client, guest, &record.tags).await {
        eprintln!("machine `{}`: tags not set yet: {error}", record.name);
        return;
    }
    if let Err(error) = database
        .activate_machine(record.environment, &record.machine)
        .await
    {
        eprintln!("machine `{}`: not marked active: {error}", record.name);
    }
}

async fn fail(database: &Database, record: &MachineRecord, failed: &Failed) {
    eprintln!(
        "machine `{}` failed: {}: {}",
        record.name, failed.reason, failed.detail
    );
    if let Err(error) = database
        .fail_machine(
            record.environment,
            &record.machine,
            &failed.reason,
            &failed.detail,
        )
        .await
    {
        eprintln!("machine `{}`: not marked failed: {error}", record.name);
    }
}

/// Looks at one pending machine's copy once, and moves the record on if it
/// is done. Used by `sweep`, for every pending record, and when someone
/// opens a pending machine's page. It doesn't wait for anything, leaves the
/// record as it is when Proxmox can't be asked, and skips one that the job
/// creating it (or another `reconcile`) has in hand.
pub async fn reconcile(database: &Database, record: &MachineRecord) {
    if record.status != MachineStatus::Pending {
        return;
    }
    let Some(_claim) = claim(record) else {
        return;
    };
    let environment = match database.proxmox_environment(record.environment).await {
        Ok(Some(environment)) => environment,
        Ok(None) => return,
        Err(error) => {
            eprintln!("machine `{}`: not looked at: {error}", record.name);
            return;
        }
    };
    let Ok(client) = environments::connect(&environment).await else {
        return;
    };
    let task = record.task.as_deref().and_then(Task::from_upid);
    let progress = match &task {
        Some(task) => match tasks::status(&client, task).await {
            Ok(progress) => Some(progress),
            // Proxmox can't be asked right now: try another time.
            Err(error)
                if matches!(
                    lemnos_proxmox::client::failure_of(&error),
                    Failure::Unreachable | Failure::LoginRejected | Failure::NotAllowed
                ) =>
            {
                return;
            }
            // Asked, and it doesn't know the task.
            Err(_) => None,
        },
        None => None,
    };

    match progress {
        Some(Progress::Running) => {}
        Some(Progress::Finished {
            outcome: Err(error),
            ..
        }) => {
            fail(
                database,
                record,
                &Failed::new("Proxmox couldn't copy the template.", error),
            )
            .await;
        }
        Some(Progress::Finished {
            outcome: Ok(()), ..
        }) => {
            if let Some(task) = &task {
                finished(database, &client, record, task).await;
            }
        }
        // No task to go by: the copy never started, or Proxmox forgot it.
        None if age(record) >= STALE_AFTER => {
            fail(
                database,
                record,
                &Failed::new(
                    "Lemnos lost track of this machine while it was being created.",
                    format!(
                        "no copy task Proxmox knows of after {} minutes (task {:?}, VM ID {:?})",
                        STALE_AFTER.as_secs() / 60,
                        record.task,
                        record.vmid
                    ),
                ),
            )
            .await;
        }
        None => {}
    }
}

/// The copy finished well: if the guest at the record's ID is what the task
/// made, tag it and mark the record active.
async fn finished(database: &Database, client: &Client, record: &MachineRecord, task: &Task) {
    let Some(vmid) = record.vmid.as_deref() else {
        return;
    };
    let guest = match guests::list(client).await {
        Ok(guests) => guests
            .into_iter()
            .find(|guest| guest.id.vmid.to_string() == vmid),
        Err(_) => return,
    };
    let Some(guest) = guest else {
        fail(
            database,
            record,
            &Failed::new(
                "The copy finished, but the machine isn't there.",
                format!("no guest with VM ID {vmid} after the copy"),
            ),
        )
        .await;
        return;
    };
    match provision::made_by(
        client,
        &guest.id,
        task,
        &record.name,
        record.marker.as_deref(),
        record.description.as_deref(),
    )
    .await
    {
        Ok(true) => finish(database, client, record, &guest.id).await,
        Ok(false) => {
            fail(
                database,
                record,
                &Failed::new(
                    "The copy finished, but a different machine has its ID.",
                    format!(
                        "guest {vmid} isn't named `{}`, carries another record's marker,                          or lacks the description its copy was given",
                        record.name
                    ),
                ),
            )
            .await;
        }
        Err(_) => {}
    }
}

/// Looks at every pending machine now and then, in the background, for as
/// long as Lemnos runs: the first time right away, which picks up copies
/// from before Lemnos last stopped, then every `SWEEP_EVERY`. With nothing
/// pending, each round is one small query.
pub fn sweep(database: Database) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(SWEEP_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            every.tick().await;
            match database.pending_machines().await {
                Ok(records) => {
                    for record in records {
                        reconcile(&database, &record).await;
                    }
                }
                Err(error) => eprintln!("pending machines not looked at: {error}"),
            }
        }
    });
}

/// The records being worked on in this process, by environment and key:
/// by the job creating them, or by a `reconcile`. Whoever comes second
/// leaves the record alone. Across processes sharing a database, the
/// record's own moves only happen while it is pending (see
/// `Database::activate_machine`), so they can't happen twice either.
static IN_HAND: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

/// A record held by [`claim`]; let go of when dropped.
pub struct Claim {
    key: (u64, String),
}

/// Takes the record for this process, unless something here has it.
pub fn claim(record: &MachineRecord) -> Option<Claim> {
    let key = (record.environment.0, record.machine.clone());
    let mut in_hand = IN_HAND.lock().unwrap_or_else(PoisonError::into_inner);
    if in_hand.contains(&key) {
        return None;
    }
    in_hand.push(key.clone());
    Some(Claim { key })
}

impl Drop for Claim {
    fn drop(&mut self) {
        IN_HAND
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|held| *held != self.key);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use lemnos_proxmox::client::StatusCode;

    use super::*;

    /// Proxmox, as far as starting a copy goes: it hands out IDs in turn,
    /// and refuses the ones listed as taken.
    struct Stub {
        ids: Mutex<Vec<u32>>,
        taken: Vec<u32>,
        refuse: bool,
        notes: Mutex<Vec<(u32, bool)>>,
    }

    impl Stub {
        fn new(ids: &[u32], taken: &[u32]) -> Self {
            Self {
                ids: Mutex::new(ids.iter().rev().copied().collect()),
                taken: taken.to_vec(),
                refuse: false,
                notes: Mutex::new(Vec::new()),
            }
        }
    }

    fn vmid(id: u32) -> VmId {
        VmId::try_from(i128::from(id)).unwrap()
    }

    impl CopySteps for Stub {
        async fn next_id(&self) -> Result<VmId, Failed> {
            let id = self
                .ids
                .lock()
                .unwrap()
                .pop()
                .expect("asked for too many IDs");
            Ok(vmid(id))
        }

        async fn note(&self, vmid: VmId, task: Option<&Task>) -> Result<(), Failed> {
            let id = vmid.to_string().parse().unwrap();
            self.notes.lock().unwrap().push((id, task.is_some()));
            Ok(())
        }

        async fn copy(&self, id: VmId) -> Result<Task, ReqwestError> {
            let number: u32 = id.to_string().parse().unwrap();
            if self.refuse {
                return Err(ReqwestError::UnknownFailure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some("storage 'local-lvm' is full".into()),
                ));
            }
            if self.taken.contains(&number) {
                return Err(ReqwestError::UnknownFailure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(format!("VM {number} already exists on node 'git'\n")),
                ));
            }
            Ok(Task::new(
                "git",
                format!("UPID:git:0:0:0:qmclone:8200:{number}:"),
            ))
        }
    }

    #[test]
    fn a_record_is_in_one_hand_at_a_time() {
        let record = MachineRecord::active(
            lemnos_db::ProxmoxEnvironmentId(1),
            "pending-lemnos-000000000001",
            "box",
            lemnos_db::Owner::Person(lemnos_auth::UserId(1)),
            None,
            None,
        );
        let first = claim(&record).expect("nobody has it yet");
        assert!(
            claim(&record).is_none(),
            "the job has it; the sweep must wait"
        );
        drop(first);
        assert!(claim(&record).is_some(), "let go of once the job is done");
    }

    #[tokio::test]
    async fn a_free_id_is_used_straight_away() {
        let stub = Stub::new(&[105], &[]);
        let (id, task) = start_copy(&stub, ATTEMPTS).await.unwrap();
        assert_eq!(id, vmid(105));
        assert_eq!(task.node, "git");
        // The ID is noted before the copy is asked for, the task after.
        assert_eq!(*stub.notes.lock().unwrap(), [(105, false), (105, true)]);
    }

    #[tokio::test]
    async fn a_taken_id_is_swapped_for_the_next() {
        let stub = Stub::new(&[105, 106, 107], &[105, 106]);
        let (id, _) = start_copy(&stub, ATTEMPTS).await.unwrap();
        assert_eq!(id, vmid(107));
        assert_eq!(
            *stub.notes.lock().unwrap(),
            [(105, false), (106, false), (107, false), (107, true)]
        );
    }

    #[tokio::test]
    async fn it_gives_up_after_the_limit() {
        let stub = Stub::new(&[105, 106, 107, 108], &[105, 106, 107, 108]);
        let failed = start_copy(&stub, ATTEMPTS).await.unwrap_err();
        assert!(failed.reason.contains("already taken"), "{failed:?}");
        assert!(failed.detail.contains("107"), "{failed:?}");
        // Three tries, no more.
        assert_eq!(stub.notes.lock().unwrap().len(), ATTEMPTS);
    }

    #[tokio::test]
    async fn any_other_refusal_is_not_retried() {
        let mut stub = Stub::new(&[105, 106], &[]);
        stub.refuse = true;
        let failed = start_copy(&stub, ATTEMPTS).await.unwrap_err();
        assert_eq!(failed.reason, "Proxmox refused to copy the template.");
        assert!(failed.detail.contains("full"));
        assert_eq!(*stub.notes.lock().unwrap(), [(105, false)]);
    }
}
