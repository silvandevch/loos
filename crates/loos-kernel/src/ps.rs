//! Process/thread scheduling state, jobs and fibers (the `Ps` layer).
//!
//! [`crate::ObjectManager`] owns object lifetime and handles; [`Ps`] owns the
//! state the scheduler needs per process and thread: priority, affinity,
//! suspend counts, job membership and fibers. Operations that must end
//! processes (job termination, kill-on-close, active-process limits) return
//! the affected [`ProcessId`]s so the caller terminates them through the
//! object manager and then reports [`Ps::process_exited`].

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::{ProcessId, ThreadId};

/// Ps failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsError {
    /// Unknown process, thread, job or fiber.
    NotFound,
    /// Identifier already registered.
    Exists,
    /// Argument outside its legal range (empty affinity, bad priority, ...).
    InvalidParameter,
    /// Suspend count would exceed [`MAX_SUSPEND`].
    SuspendLimit,
    /// The process already belongs to a job.
    AlreadyInJob,
    /// The job's active-process limit is reached.
    LimitExceeded,
    /// The thread is not a fiber thread, or already is one.
    FiberState,
    /// The fiber is running on another thread.
    FiberBusy,
}

/// Highest suspend count (`MAXIMUM_SUSPEND_COUNT`).
pub const MAX_SUSPEND: u8 = 127;

/// Process priority class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityClass {
    /// Base 4.
    Idle,
    /// Base 6.
    BelowNormal,
    /// Base 8.
    Normal,
    /// Base 10.
    AboveNormal,
    /// Base 13.
    High,
    /// Base 24; realtime range 16..=31.
    Realtime,
}

impl PriorityClass {
    /// Base priority of the class.
    pub fn base(self) -> i8 {
        match self {
            Self::Idle => 4,
            Self::BelowNormal => 6,
            Self::Normal => 8,
            Self::AboveNormal => 10,
            Self::High => 13,
            Self::Realtime => 24,
        }
    }
}

/// `THREAD_PRIORITY_IDLE`: saturates to 1 (16 for realtime).
pub const THREAD_PRIORITY_IDLE: i8 = -15;
/// `THREAD_PRIORITY_TIME_CRITICAL`: saturates to 15 (31 for realtime).
pub const THREAD_PRIORITY_TIME_CRITICAL: i8 = 15;

/// Effective base priority for a thread with relative priority `rel` (-2..=2,
/// or the idle/time-critical saturation values) in `class`.
pub fn thread_base_priority(class: PriorityClass, rel: i8) -> i8 {
    let rt = class == PriorityClass::Realtime;
    match rel {
        THREAD_PRIORITY_IDLE => if rt { 16 } else { 1 },
        THREAD_PRIORITY_TIME_CRITICAL => if rt { 31 } else { 15 },
        r => {
            let p = class.base() + r;
            if rt { p.clamp(16, 31) } else { p.clamp(1, 15) }
        }
    }
}

/// Job identifier.
pub type JobId = u64;
/// Fiber identifier.
pub type FiberId = u64;

/// Job limits (`JOBOBJECT_BASIC_LIMIT_INFORMATION` subset).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JobLimits {
    /// Maximum simultaneously active processes; `None` = unlimited.
    pub active_processes: Option<u32>,
    /// Affinity forced on every member; `None` = unrestricted.
    pub affinity: Option<u64>,
    /// Priority class forced on every member.
    pub priority_class: Option<PriorityClass>,
    /// Terminate all members when the last job handle closes.
    pub kill_on_close: bool,
    /// Children may be created outside the job when asked.
    pub breakaway_ok: bool,
}

/// Accounting snapshot of a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobInfo {
    /// Limits in force.
    pub limits: JobLimits,
    /// Live member processes.
    pub active: Vec<ProcessId>,
    /// Processes ever assigned.
    pub total_processes: u32,
    /// Processes that exited or were terminated.
    pub terminated_processes: u32,
}

struct Process {
    class: PriorityClass,
    affinity: u64,
    job: Option<JobId>,
    threads: Vec<ThreadId>,
}

struct Thread {
    process: ProcessId,
    relative: i8,
    affinity: u64,
    suspend: u8,
    fiber: Option<FiberId>,
}

struct Job {
    limits: JobLimits,
    active: Vec<ProcessId>,
    total: u32,
    terminated: u32,
    closed: bool,
}

struct Fiber {
    process: ProcessId,
    start: u64,
    param: u64,
    stack_size: u64,
    running_on: Option<ThreadId>,
}

/// Snapshot of a fiber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiberInfo {
    /// Entry point (0 for a converted thread).
    pub start: u64,
    /// `lpParameter`.
    pub param: u64,
    /// Reserved stack size in bytes (0 = thread's own stack).
    pub stack_size: u64,
    /// Thread currently running it.
    pub running_on: Option<ThreadId>,
}

/// Process/thread/job/fiber state for the whole system.
pub struct Ps {
    processors: u64,
    processes: BTreeMap<ProcessId, Process>,
    threads: BTreeMap<ThreadId, Thread>,
    jobs: BTreeMap<JobId, Job>,
    fibers: BTreeMap<FiberId, Fiber>,
    next_job: JobId,
    next_fiber: FiberId,
}

impl Ps {
    /// Creates the Ps layer for a machine with `cpus` processors (1..=64).
    pub const fn new(cpus: u32) -> Self {
        let cpus = if cpus == 0 { 1 } else if cpus > 64 { 64 } else { cpus };
        Self {
            processors: if cpus == 64 { u64::MAX } else { (1u64 << cpus) - 1 },
            processes: BTreeMap::new(),
            threads: BTreeMap::new(),
            jobs: BTreeMap::new(),
            fibers: BTreeMap::new(),
            next_job: 1,
            next_fiber: 1,
        }
    }

    /// Mask of every processor in the system.
    pub fn system_affinity(&self) -> u64 {
        self.processors
    }

    /// Registers a process created by the object manager. A child inherits
    /// its parent's priority class and job unless `breakaway` is requested
    /// and the job allows it.
    pub fn process_created(&mut self, id: ProcessId, parent: Option<ProcessId>, breakaway: bool) -> Result<(), PsError> {
        if self.processes.contains_key(&id) {
            return Err(PsError::Exists);
        }
        let (class, job) = match parent.and_then(|p| self.processes.get(&p)) {
            Some(p) => (if p.class == PriorityClass::Idle { PriorityClass::Idle } else { PriorityClass::Normal }, p.job),
            None => (PriorityClass::Normal, None),
        };
        let job = job.filter(|j| !(breakaway && self.jobs.get(j).is_some_and(|j| j.limits.breakaway_ok)));
        self.processes.insert(id, Process { class, affinity: self.processors, job: None, threads: Vec::new() });
        if let Some(j) = job {
            if let Err(e) = self.assign(j, id) {
                self.processes.remove(&id);
                return Err(e);
            }
        }
        Ok(())
    }

    /// Registers a thread. `suspended` is `CREATE_SUSPENDED`.
    pub fn thread_created(&mut self, id: ThreadId, process: ProcessId, suspended: bool) -> Result<(), PsError> {
        if self.threads.contains_key(&id) {
            return Err(PsError::Exists);
        }
        let p = self.processes.get_mut(&process).ok_or(PsError::NotFound)?;
        p.threads.push(id);
        let affinity = p.affinity;
        self.threads.insert(id, Thread { process, relative: 0, affinity, suspend: suspended as u8, fiber: None });
        Ok(())
    }

    /// Forgets an exited thread and its converted-thread fiber. Fibers it
    /// created stay with the process until deleted.
    pub fn thread_exited(&mut self, id: ThreadId) {
        if let Some(t) = self.threads.remove(&id) {
            if let Some(f) = t.fiber.and_then(|f| self.fibers.get_mut(&f)) {
                f.running_on = None;
            }
            if let Some(p) = self.processes.get_mut(&t.process) {
                p.threads.retain(|&x| x != id);
            }
        }
    }

    /// Forgets an exited process, its threads and fibers, and updates job accounting.
    pub fn process_exited(&mut self, id: ProcessId) {
        let Some(p) = self.processes.remove(&id) else { return };
        for t in p.threads {
            self.threads.remove(&t);
        }
        self.fibers.retain(|_, f| f.process != id);
        if let Some(jid) = p.job {
            if let Some(j) = self.jobs.get_mut(&jid) {
                j.active.retain(|&x| x != id);
                j.terminated += 1;
                if j.closed && j.active.is_empty() {
                    self.jobs.remove(&jid);
                }
            }
        }
    }

    fn process(&self, id: ProcessId) -> Result<&Process, PsError> {
        self.processes.get(&id).ok_or(PsError::NotFound)
    }

    fn thread(&self, id: ThreadId) -> Result<&Thread, PsError> {
        self.threads.get(&id).ok_or(PsError::NotFound)
    }

    fn thread_mut(&mut self, id: ThreadId) -> Result<&mut Thread, PsError> {
        self.threads.get_mut(&id).ok_or(PsError::NotFound)
    }

    // ---- priority and affinity ----

    /// `SetPriorityClass`. A job-imposed class wins.
    pub fn set_priority_class(&mut self, id: ProcessId, class: PriorityClass) -> Result<(), PsError> {
        let forced = self.process(id)?.job.and_then(|j| self.jobs.get(&j)).and_then(|j| j.limits.priority_class);
        self.processes.get_mut(&id).ok_or(PsError::NotFound)?.class = forced.unwrap_or(class);
        Ok(())
    }

    /// Process priority class.
    pub fn priority_class(&self, id: ProcessId) -> Result<PriorityClass, PsError> {
        Ok(self.process(id)?.class)
    }

    /// `SetThreadPriority` with a relative value (-2..=2, idle or time-critical).
    pub fn set_thread_priority(&mut self, id: ThreadId, rel: i8) -> Result<(), PsError> {
        if !matches!(rel, -2..=2 | THREAD_PRIORITY_IDLE | THREAD_PRIORITY_TIME_CRITICAL) {
            return Err(PsError::InvalidParameter);
        }
        self.thread_mut(id)?.relative = rel;
        Ok(())
    }

    /// Scheduler base priority (1..=31) of a thread.
    pub fn base_priority(&self, id: ThreadId) -> Result<i8, PsError> {
        let t = self.thread(id)?;
        Ok(thread_base_priority(self.process(t.process)?.class, t.relative))
    }

    /// `SetProcessAffinityMask`: must be a non-empty subset of the system and
    /// of any job mask. Thread masks are narrowed to it (reset to it if that
    /// would leave them empty).
    pub fn set_process_affinity(&mut self, id: ProcessId, mask: u64) -> Result<(), PsError> {
        let p = self.process(id)?;
        let limit = p.job.and_then(|j| self.jobs.get(&j)).and_then(|j| j.limits.affinity).unwrap_or(u64::MAX) & self.processors;
        if mask == 0 || mask & !limit != 0 {
            return Err(PsError::InvalidParameter);
        }
        self.apply_process_affinity(id, mask);
        Ok(())
    }

    fn apply_process_affinity(&mut self, id: ProcessId, mask: u64) {
        let Some(p) = self.processes.get_mut(&id) else { return };
        p.affinity = mask;
        for t in &p.threads {
            if let Some(t) = self.threads.get_mut(t) {
                t.affinity = if t.affinity & mask == 0 { mask } else { t.affinity & mask };
            }
        }
    }

    /// `SetThreadAffinityMask`: returns the previous mask.
    pub fn set_thread_affinity(&mut self, id: ThreadId, mask: u64) -> Result<u64, PsError> {
        let allowed = self.process(self.thread(id)?.process)?.affinity;
        if mask == 0 || mask & !allowed != 0 {
            return Err(PsError::InvalidParameter);
        }
        Ok(core::mem::replace(&mut self.thread_mut(id)?.affinity, mask))
    }

    /// Thread affinity mask.
    pub fn thread_affinity(&self, id: ThreadId) -> Result<u64, PsError> {
        Ok(self.thread(id)?.affinity)
    }

    // ---- suspend / resume ----

    /// `SuspendThread`: returns the previous count.
    pub fn suspend(&mut self, id: ThreadId) -> Result<u8, PsError> {
        let t = self.thread_mut(id)?;
        if t.suspend >= MAX_SUSPEND {
            return Err(PsError::SuspendLimit);
        }
        t.suspend += 1;
        Ok(t.suspend - 1)
    }

    /// `ResumeThread`: returns the previous count; the thread is runnable when it reaches 0.
    pub fn resume(&mut self, id: ThreadId) -> Result<u8, PsError> {
        let t = self.thread_mut(id)?;
        let prev = t.suspend;
        t.suspend = prev.saturating_sub(1);
        Ok(prev)
    }

    /// Whether the scheduler may run the thread.
    pub fn runnable(&self, id: ThreadId) -> Result<bool, PsError> {
        Ok(self.thread(id)?.suspend == 0)
    }

    // ---- jobs ----

    /// `CreateJobObject` + `SetInformationJobObject`.
    pub fn create_job(&mut self, limits: JobLimits) -> Result<JobId, PsError> {
        if limits.affinity.is_some_and(|m| m & self.processors == 0) {
            return Err(PsError::InvalidParameter);
        }
        let id = self.next_job;
        self.next_job += 1;
        self.jobs.insert(id, Job { limits, active: Vec::new(), total: 0, terminated: 0, closed: false });
        Ok(id)
    }

    /// `AssignProcessToJobObject`; applies the job's class and affinity limits.
    pub fn assign(&mut self, job: JobId, process: ProcessId) -> Result<(), PsError> {
        if self.process(process)?.job.is_some() {
            return Err(PsError::AlreadyInJob);
        }
        let j = self.jobs.get_mut(&job).filter(|j| !j.closed).ok_or(PsError::NotFound)?;
        if j.limits.active_processes.is_some_and(|n| j.active.len() as u32 >= n) {
            return Err(PsError::LimitExceeded);
        }
        j.active.push(process);
        j.total += 1;
        let limits = j.limits;
        let p = self.processes.get_mut(&process).ok_or(PsError::NotFound)?;
        p.job = Some(job);
        if let Some(c) = limits.priority_class {
            p.class = c;
        }
        if let Some(m) = limits.affinity {
            let mask = p.affinity & m & self.processors;
            self.apply_process_affinity(process, if mask == 0 { m & self.processors } else { mask });
        }
        Ok(())
    }

    /// Job a process belongs to.
    pub fn job_of(&self, process: ProcessId) -> Result<Option<JobId>, PsError> {
        Ok(self.process(process)?.job)
    }

    /// Whether a job still exists (open, or closed with live members).
    pub fn job_exists(&self, job: JobId) -> bool {
        self.jobs.contains_key(&job)
    }

    /// Job accounting snapshot.
    pub fn job_info(&self, job: JobId) -> Result<JobInfo, PsError> {
        let j = self.jobs.get(&job).ok_or(PsError::NotFound)?;
        Ok(JobInfo { limits: j.limits, active: j.active.clone(), total_processes: j.total, terminated_processes: j.terminated })
    }

    /// `TerminateJobObject`: the processes the caller must terminate.
    pub fn terminate_job(&self, job: JobId) -> Result<Vec<ProcessId>, PsError> {
        Ok(self.jobs.get(&job).ok_or(PsError::NotFound)?.active.clone())
    }

    /// Last handle to the job closed. Returns processes to kill when
    /// `kill_on_close` is set. The job is freed once its last member exits.
    pub fn close_job(&mut self, job: JobId) -> Result<Vec<ProcessId>, PsError> {
        let j = self.jobs.get_mut(&job).ok_or(PsError::NotFound)?;
        j.closed = true;
        let kill = if j.limits.kill_on_close { j.active.clone() } else { Vec::new() };
        if j.active.is_empty() {
            self.jobs.remove(&job);
        }
        Ok(kill)
    }

    // ---- fibers ----

    /// `ConvertThreadToFiber`: the thread now runs a fiber wrapping itself.
    pub fn convert_thread_to_fiber(&mut self, thread: ThreadId, param: u64) -> Result<FiberId, PsError> {
        let t = self.thread(thread)?;
        if t.fiber.is_some() {
            return Err(PsError::FiberState);
        }
        let process = t.process;
        let id = self.alloc_fiber(Fiber { process, start: 0, param, stack_size: 0, running_on: Some(thread) });
        self.thread_mut(thread)?.fiber = Some(id);
        Ok(id)
    }

    /// `CreateFiber` in `process`; the fiber is not running anywhere.
    pub fn create_fiber(&mut self, process: ProcessId, stack_size: u64, start: u64, param: u64) -> Result<FiberId, PsError> {
        self.process(process)?;
        if start == 0 {
            return Err(PsError::InvalidParameter);
        }
        Ok(self.alloc_fiber(Fiber { process, start, param, stack_size, running_on: None }))
    }

    fn alloc_fiber(&mut self, f: Fiber) -> FiberId {
        let id = self.next_fiber;
        self.next_fiber += 1;
        self.fibers.insert(id, f);
        id
    }

    /// `SwitchToFiber`: returns the fiber the thread left. The target must be
    /// in the same process and not running on another thread.
    pub fn switch_to_fiber(&mut self, thread: ThreadId, target: FiberId) -> Result<FiberId, PsError> {
        let t = self.thread(thread)?;
        let cur = t.fiber.ok_or(PsError::FiberState)?;
        let f = self.fibers.get(&target).ok_or(PsError::NotFound)?;
        if f.process != t.process {
            return Err(PsError::InvalidParameter);
        }
        if f.running_on.is_some_and(|r| r != thread) {
            return Err(PsError::FiberBusy);
        }
        if let Some(c) = self.fibers.get_mut(&cur) {
            c.running_on = None;
        }
        self.fibers.get_mut(&target).ok_or(PsError::NotFound)?.running_on = Some(thread);
        self.thread_mut(thread)?.fiber = Some(target);
        Ok(cur)
    }

    /// Fiber the thread is running, if it is a fiber thread.
    pub fn current_fiber(&self, thread: ThreadId) -> Result<Option<FiberId>, PsError> {
        Ok(self.thread(thread)?.fiber)
    }

    /// Fiber snapshot.
    pub fn fiber_info(&self, id: FiberId) -> Result<FiberInfo, PsError> {
        let f = self.fibers.get(&id).ok_or(PsError::NotFound)?;
        Ok(FiberInfo { start: f.start, param: f.param, stack_size: f.stack_size, running_on: f.running_on })
    }

    /// `DeleteFiber`. Returns the thread that must exit because it deleted
    /// the fiber it was running (Windows calls `ExitThread` in that case).
    pub fn delete_fiber(&mut self, id: FiberId) -> Result<Option<ThreadId>, PsError> {
        let f = self.fibers.remove(&id).ok_or(PsError::NotFound)?;
        if let Some(t) = f.running_on.and_then(|t| self.threads.get_mut(&t)) {
            t.fiber = None;
        }
        Ok(f.running_on)
    }

    /// `ConvertFiberToThread`: frees the current fiber; the thread is plain again.
    pub fn convert_fiber_to_thread(&mut self, thread: ThreadId) -> Result<(), PsError> {
        let f = self.thread_mut(thread)?.fiber.take().ok_or(PsError::FiberState)?;
        self.fibers.remove(&f);
        Ok(())
    }
}
