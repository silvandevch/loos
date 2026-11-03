//! Host-testable executive objects and the first LoOS system-call surface.
//!
//! The manager is independent of architecture entry code. The x86-64 syscall
//! stub can validate register arguments and pass them to [`dispatch`]; lifetime
//! and access checks happen here. Handles contain an index and generation, so
//! closing and reusing a slot cannot revive a stale user value.
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub mod ps;
pub mod seh;
pub mod smss;

/// Rights understood by the first object syscalls.
pub mod access {
    /// Inspect object state.
    pub const QUERY: u32 = 0x0001;
    /// Change event state.
    pub const MODIFY_STATE: u32 = 0x0002;
    /// Wait for an object.
    pub const SYNCHRONIZE: u32 = 0x0004;
    /// Terminate a process or thread.
    pub const TERMINATE: u32 = 0x0008;
    /// Duplicate a handle.
    pub const DUPLICATE: u32 = 0x0010;
    /// Replace the object's security descriptor.
    pub const SET_SECURITY: u32 = 0x0020;
    /// All rights supported by this crate.
    pub const ALL: u32 = QUERY | MODIFY_STATE | SYNCHRONIZE | TERMINATE | DUPLICATE | SET_SECURITY;
}

/// Object kinds exposed by the executive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectType {
    /// A process address-space owner.
    Process = 1,
    /// A schedulable thread belonging to a process.
    Thread = 2,
    /// A notification or synchronization event.
    Event = 3,
    /// A job grouping processes under shared limits.
    Job = 4,
}

/// Event reset behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Remains signaled until reset; all eligible waiters may observe it.
    Notification,
    /// A successful wait consumes the signal.
    Synchronization,
}

/// An opaque user-visible handle.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle {
    index: u32,
    generation: u32,
}

impl Handle {
    /// The invalid handle value.
    pub const INVALID: Self = Self {
        index: u32::MAX,
        generation: 0,
    };

    /// Encodes the handle for a syscall argument or ABI structure.
    pub const fn raw(self) -> u64 {
        ((self.generation as u64) << 32) | (self.index as u64 + 1)
    }

    /// Decodes a raw syscall handle. Lifetime validity is checked by the manager.
    pub const fn from_raw(raw: u64) -> Option<Self> {
        let low = raw as u32;
        if low == 0 || raw >> 32 > u32::MAX as u64 {
            return None;
        }
        Some(Self {
            index: low - 1,
            generation: (raw >> 32) as u32,
        })
    }
}

impl core::fmt::Debug for Handle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handle").field("raw", &self.raw()).finish()
    }
}

/// Stable process identifier.
pub type ProcessId = u64;
/// Stable thread identifier.
pub type ThreadId = u64;

/// Security principal identifier used by the object ACL model.
pub type PrincipalId = u64;

/// A compact discretionary access descriptor for an executive object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecurityDescriptor {
    /// Principal that created/owns the object.
    pub owner: PrincipalId,
    /// Rights granted to authorized callers.
    pub allow: u32,
    /// Rights explicitly denied before `allow` is considered.
    pub deny: u32,
}

impl SecurityDescriptor {
    /// Creates a descriptor with explicit owner, allow, and deny masks.
    pub const fn new(owner: PrincipalId, allow: u32, deny: u32) -> Self {
        Self { owner, allow, deny }
    }

    /// Descriptor for an object initially accessible with every supported right.
    pub const fn allow_all(owner: PrincipalId) -> Self {
        Self::new(owner, access::ALL, 0)
    }

    fn check(self, principal: PrincipalId, rights: u32) -> Result<(), Error> {
        // Principal zero is the built-in unrestricted/system identity. A
        // non-zero owner descriptor is private to its owner until a future
        // ACL implementation adds group/world entries.
        if self.owner != 0 && principal != self.owner {
            return Err(Error::AccessDenied);
        }
        if self.deny & rights != 0 || self.allow & rights != rights {
            return Err(Error::AccessDenied);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectNamespace {
    Unnamed,
    BaseNamedObjects,
}

/// Executive failures. These are mapped to syscall statuses by [`dispatch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A handle is malformed, stale, or closed.
    InvalidHandle,
    /// The handle names another object kind.
    WrongType,
    /// The requested operation is not allowed by the handle's rights.
    AccessDenied,
    /// No object with the requested name or identifier exists.
    NotFound,
    /// A supplied name is empty.
    InvalidName,
    /// A process/thread has already terminated.
    AlreadyTerminated,
    /// An operation would exceed the supported identifier range.
    LimitExceeded,
    /// Bad argument or state for a Ps operation (fiber, job, affinity).
    InvalidState,
}

impl From<ps::PsError> for Error {
    fn from(e: ps::PsError) -> Self {
        use ps::PsError::*;
        match e {
            NotFound => Self::NotFound,
            LimitExceeded | SuspendLimit => Self::LimitExceeded,
            Exists | InvalidParameter | AlreadyInJob | FiberState | FiberBusy => Self::InvalidState,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectRef {
    index: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HandleFlags {
    inherit: bool,
}

struct HandleSlot {
    generation: u32,
    object: Option<ObjectRef>,
    rights: u32,
    flags: HandleFlags,
}

struct ProcessObject {
    id: ProcessId,
    parent: Option<ProcessId>,
    terminated: bool,
    exit_status: u32,
    threads: Vec<ThreadId>,
}

struct ThreadObject {
    id: ThreadId,
    process: ProcessId,
    terminated: bool,
    exit_status: u32,
}

struct EventObject {
    kind: EventKind,
    signaled: bool,
}

enum ObjectKind {
    Process(ProcessObject),
    Thread(ThreadObject),
    Event(EventObject),
    Job(ps::JobId),
}

struct ObjectSlot {
    generation: u32,
    references: u32,
    name: Option<String>,
    namespace: ObjectNamespace,
    security: SecurityDescriptor,
    object: Option<ObjectKind>,
}

/// Snapshot of a process safe to return across the syscall boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Process identifier.
    pub id: ProcessId,
    /// Parent process identifier, if any.
    pub parent: Option<ProcessId>,
    /// Whether termination has completed.
    pub terminated: bool,
    /// Exit status (meaningful after termination).
    pub exit_status: u32,
    /// Number of threads currently attached to the process.
    pub thread_count: usize,
}

/// Snapshot of a thread safe to return across the syscall boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadInfo {
    /// Thread identifier.
    pub id: ThreadId,
    /// Owning process identifier.
    pub process: ProcessId,
    /// Whether termination has completed.
    pub terminated: bool,
    /// Exit status (meaningful after termination).
    pub exit_status: u32,
}

/// Snapshot of an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventInfo {
    /// Reset behavior.
    pub kind: EventKind,
    /// Current signal state.
    pub signaled: bool,
}

/// Kernel object manager and per-process handle table.
pub struct ObjectManager {
    handles: Vec<HandleSlot>,
    free_handles: Vec<u32>,
    objects: Vec<ObjectSlot>,
    free_objects: Vec<u32>,
    principal: PrincipalId,
    next_process: ProcessId,
    next_thread: ThreadId,
    /// Scheduler-facing process/thread/job/fiber state, kept in step with objects.
    pub ps: ps::Ps,
}

impl Default for ObjectManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectManager {
    /// Creates an empty manager. Identifiers start at one.
    pub const fn new() -> Self {
        Self::with_cpus(1)
    }

    /// Creates a manager for a machine with `cpus` processors.
    pub const fn with_cpus(cpus: u32) -> Self {
        Self {
            handles: Vec::new(),
            free_handles: Vec::new(),
            objects: Vec::new(),
            free_objects: Vec::new(),
            principal: 0,
            next_process: 1,
            next_thread: 1,
            ps: ps::Ps::new(cpus),
        }
    }

    fn alloc_object(&mut self, object: ObjectKind, name: Option<String>) -> ObjectRef {
        self.alloc_object_with_security(object, name, SecurityDescriptor::allow_all(self.principal))
    }

    fn alloc_object_with_security(
        &mut self,
        object: ObjectKind,
        name: Option<String>,
        security: SecurityDescriptor,
    ) -> ObjectRef {
        let namespace = if name.is_some() {
            ObjectNamespace::BaseNamedObjects
        } else {
            ObjectNamespace::Unnamed
        };
        if let Some(index) = self.free_objects.pop() {
            let slot = &mut self.objects[index as usize];
            slot.object = Some(object);
            slot.references = 0;
            slot.name = name;
            slot.namespace = namespace;
            slot.security = security;
            return ObjectRef {
                index,
                generation: slot.generation,
            };
        }
        let index = self.objects.len() as u32;
        self.objects.push(ObjectSlot {
            generation: 1,
            references: 0,
            name,
            namespace,
            security,
            object: Some(object),
        });
        ObjectRef {
            index,
            generation: 1,
        }
    }

    fn alloc_handle(&mut self, object: ObjectRef, rights: u32, inherit: bool) -> Handle {
        if let Some(index) = self.free_handles.pop() {
            let slot = &mut self.handles[index as usize];
            slot.object = Some(object);
            slot.rights = rights;
            slot.flags = HandleFlags { inherit };
            self.objects[object.index as usize].references += 1;
            return Handle {
                index,
                generation: slot.generation,
            };
        }
        let index = self.handles.len() as u32;
        self.handles.push(HandleSlot {
            generation: 1,
            object: Some(object),
            rights,
            flags: HandleFlags { inherit },
        });
        self.objects[object.index as usize].references += 1;
        Handle {
            index,
            generation: 1,
        }
    }

    fn object_ref(&self, handle: Handle) -> Result<ObjectRef, Error> {
        let slot = self
            .handles
            .get(handle.index as usize)
            .ok_or(Error::InvalidHandle)?;
        if slot.generation != handle.generation {
            return Err(Error::InvalidHandle);
        }
        let object = slot.object.ok_or(Error::InvalidHandle)?;
        let object_slot = self
            .objects
            .get(object.index as usize)
            .ok_or(Error::InvalidHandle)?;
        if object_slot.generation != object.generation || object_slot.object.is_none() {
            return Err(Error::InvalidHandle);
        }
        Ok(object)
    }

    fn require(&self, handle: Handle, rights: u32) -> Result<ObjectRef, Error> {
        let object = self.object_ref(handle)?;
        if self.handles[handle.index as usize].rights & rights != rights {
            return Err(Error::AccessDenied);
        }
        self.check_security(object, rights)?;
        Ok(object)
    }

    fn check_security(&self, object: ObjectRef, rights: u32) -> Result<(), Error> {
        let slot = self
            .objects
            .get(object.index as usize)
            .ok_or(Error::InvalidHandle)?;
        if slot.generation != object.generation {
            return Err(Error::InvalidHandle);
        }
        slot.security.check(self.principal, rights)
    }

    fn object(&self, object: ObjectRef) -> Result<&ObjectKind, Error> {
        let slot = self
            .objects
            .get(object.index as usize)
            .ok_or(Error::InvalidHandle)?;
        if slot.generation != object.generation {
            return Err(Error::InvalidHandle);
        }
        slot.object.as_ref().ok_or(Error::InvalidHandle)
    }

    fn object_mut(&mut self, object: ObjectRef) -> Result<&mut ObjectKind, Error> {
        let slot = self
            .objects
            .get_mut(object.index as usize)
            .ok_or(Error::InvalidHandle)?;
        if slot.generation != object.generation {
            return Err(Error::InvalidHandle);
        }
        slot.object.as_mut().ok_or(Error::InvalidHandle)
    }

    fn find_named(&self, name: &str, kind: ObjectType) -> Option<ObjectRef> {
        let (name, namespace) = normalize_named_path(name).ok()?;
        self.objects.iter().enumerate().find_map(|(i, slot)| {
            if slot.name.as_deref() != Some(name.as_str()) || slot.namespace != namespace {
                return None;
            }
            let object = slot.object.as_ref()?;
            let matches = matches!(
                (kind, object),
                (ObjectType::Event, ObjectKind::Event(_))
                    | (ObjectType::Process, ObjectKind::Process(_))
                    | (ObjectType::Thread, ObjectKind::Thread(_))
                    | (ObjectType::Job, ObjectKind::Job(_))
            );
            matches.then_some(ObjectRef {
                index: i as u32,
                generation: slot.generation,
            })
        })
    }

    fn add_object_handle(&mut self, object: ObjectRef) -> Handle {
        self.alloc_handle(object, access::ALL, false)
    }

    /// Changes the principal used for subsequent object access checks.
    pub fn set_principal(&mut self, principal: PrincipalId) {
        self.principal = principal;
    }

    /// Returns the principal currently used for access checks.
    pub fn principal(&self) -> PrincipalId {
        self.principal
    }

    /// Creates a process and returns its full-access handle and stable identifier.
    /// Fails with `LimitExceeded` when the parent's job is full.
    pub fn create_process(&mut self, parent: Option<ProcessId>) -> Result<(Handle, ProcessId), Error> {
        self.create_process_ex(parent, false)
    }

    /// [`Self::create_process`] with `CREATE_BREAKAWAY_FROM_JOB`.
    pub fn create_process_ex(&mut self, parent: Option<ProcessId>, breakaway: bool) -> Result<(Handle, ProcessId), Error> {
        let id = self.next_process;
        self.ps.process_created(id, parent, breakaway).map_err(Error::from)?;
        self.next_process = self.next_process.saturating_add(1).max(1);
        let object = self.alloc_object(
            ObjectKind::Process(ProcessObject {
                id,
                parent,
                terminated: false,
                exit_status: 0,
                threads: Vec::new(),
            }),
            None,
        );
        Ok((self.add_object_handle(object), id))
    }

    /// Creates a thread attached to the process named by `process`.
    pub fn create_thread(&mut self, process: Handle) -> Result<(Handle, ThreadId), Error> {
        self.create_thread_ex(process, false)
    }

    /// [`Self::create_thread`] with `CREATE_SUSPENDED`.
    pub fn create_thread_ex(&mut self, process: Handle, suspended: bool) -> Result<(Handle, ThreadId), Error> {
        let object = self.require(process, access::QUERY)?;
        let process_id = match self.object(object)? {
            ObjectKind::Process(p) if !p.terminated => p.id,
            ObjectKind::Process(_) => return Err(Error::AlreadyTerminated),
            _ => return Err(Error::WrongType),
        };
        let id = self.next_thread;
        self.ps.thread_created(id, process_id, suspended).map_err(Error::from)?;
        self.next_thread = self.next_thread.saturating_add(1).max(1);
        let thread_object = self.alloc_object(
            ObjectKind::Thread(ThreadObject {
                id,
                process: process_id,
                terminated: false,
                exit_status: 0,
            }),
            None,
        );
        if let ObjectKind::Process(p) = self.object_mut(object)? {
            p.threads.push(id);
        }
        Ok((self.add_object_handle(thread_object), id))
    }

    /// Creates an unnamed event.
    pub fn create_event(&mut self, kind: EventKind, signaled: bool) -> Handle {
        let object = self.alloc_object(ObjectKind::Event(EventObject { kind, signaled }), None);
        self.add_object_handle(object)
    }

    /// Creates or opens a named event in the manager's local namespace.
    pub fn create_named_event(
        &mut self,
        name: &str,
        kind: EventKind,
        signaled: bool,
    ) -> Result<(Handle, bool), Error> {
        self.create_named_event_with_security(
            name,
            kind,
            signaled,
            SecurityDescriptor::allow_all(self.principal),
        )
    }

    /// Creates or opens a named event with an explicit security descriptor.
    pub fn create_named_event_with_security(
        &mut self,
        name: &str,
        kind: EventKind,
        signaled: bool,
        security: SecurityDescriptor,
    ) -> Result<(Handle, bool), Error> {
        let (canonical, _) = normalize_named_path(name)?;
        if let Some(object) = self.find_named(name, ObjectType::Event) {
            self.check_security(object, access::ALL)?;
            return Ok((self.add_object_handle(object), false));
        }
        let object = self.alloc_object_with_security(
            ObjectKind::Event(EventObject { kind, signaled }),
            Some(canonical),
            security,
        );
        Ok((self.add_object_handle(object), true))
    }

    /// Opens an existing named event.
    pub fn open_named_event(&mut self, name: &str) -> Result<Handle, Error> {
        let object = self
            .find_named(name, ObjectType::Event)
            .ok_or(Error::NotFound)?;
        self.check_security(object, access::ALL)?;
        Ok(self.add_object_handle(object))
    }

    /// Replaces an object's descriptor after checking `SET_SECURITY` access.
    pub fn set_security_descriptor(
        &mut self,
        handle: Handle,
        security: SecurityDescriptor,
    ) -> Result<(), Error> {
        let object = self.require(handle, access::SET_SECURITY)?;
        self.objects[object.index as usize].security = security;
        Ok(())
    }

    /// Reads an object's descriptor after checking query access.
    pub fn security_descriptor(&self, handle: Handle) -> Result<SecurityDescriptor, Error> {
        let object = self.require(handle, access::QUERY)?;
        Ok(self.objects[object.index as usize].security)
    }

    /// Finds a process by identifier and grants a full-access handle.
    pub fn open_process(&mut self, id: ProcessId) -> Result<Handle, Error> {
        let object = self
            .objects
            .iter()
            .enumerate()
            .find_map(|(i, slot)| match slot.object.as_ref()? {
                ObjectKind::Process(p) if p.id == id => Some(ObjectRef {
                    index: i as u32,
                    generation: slot.generation,
                }),
                _ => None,
            })
            .ok_or(Error::NotFound)?;
        Ok(self.add_object_handle(object))
    }

    /// Finds a thread by identifier and grants a full-access handle.
    pub fn open_thread(&mut self, id: ThreadId) -> Result<Handle, Error> {
        let object = self
            .objects
            .iter()
            .enumerate()
            .find_map(|(i, slot)| match slot.object.as_ref()? {
                ObjectKind::Thread(t) if t.id == id => Some(ObjectRef {
                    index: i as u32,
                    generation: slot.generation,
                }),
                _ => None,
            })
            .ok_or(Error::NotFound)?;
        Ok(self.add_object_handle(object))
    }

    /// Closes a handle and invalidates its raw value permanently.
    pub fn close(&mut self, handle: Handle) -> Result<(), Error> {
        let object = self.object_ref(handle)?;
        let slot = &mut self.handles[handle.index as usize];
        slot.object = None;
        slot.rights = 0;
        slot.flags = HandleFlags { inherit: false };
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free_handles.push(handle.index);
        let object_slot = &mut self.objects[object.index as usize];
        object_slot.references -= 1;
        if object_slot.references == 0 {
            if let Some(ObjectKind::Job(j)) = object_slot.object {
                for pid in self.ps.close_job(j).unwrap_or_default() {
                    self.terminate_process_id(pid, 0);
                }
            }
            let object_slot = &mut self.objects[object.index as usize];
            object_slot.object = None;
            object_slot.name = None;
            object_slot.generation = object_slot.generation.wrapping_add(1).max(1);
            self.free_objects.push(object.index);
        }
        Ok(())
    }

    /// Duplicates a handle. `rights == 0` means the source rights; otherwise
    /// requested rights must be a subset of the source rights.
    pub fn duplicate(
        &mut self,
        source: Handle,
        rights: u32,
        inherit: bool,
    ) -> Result<Handle, Error> {
        let object = self.require(source, access::DUPLICATE)?;
        let source_rights = self.handles[source.index as usize].rights;
        let rights = if rights == 0 { source_rights } else { rights };
        if rights & !source_rights != 0 {
            return Err(Error::AccessDenied);
        }
        self.check_security(object, rights)?;
        Ok(self.alloc_handle(object, rights, inherit))
    }

    /// Reads the rights and inherit bit associated with a handle.
    pub fn handle_rights(&self, handle: Handle) -> Result<(u32, bool), Error> {
        self.object_ref(handle)?;
        let slot = &self.handles[handle.index as usize];
        Ok((slot.rights, slot.flags.inherit))
    }

    /// Returns the object's type after validating the handle.
    pub fn object_type(&self, handle: Handle) -> Result<ObjectType, Error> {
        match self.object(self.object_ref(handle)?)? {
            ObjectKind::Process(_) => Ok(ObjectType::Process),
            ObjectKind::Thread(_) => Ok(ObjectType::Thread),
            ObjectKind::Event(_) => Ok(ObjectType::Event),
            ObjectKind::Job(_) => Ok(ObjectType::Job),
        }
    }

    /// Returns process state after checking query access.
    pub fn process_info(&self, handle: Handle) -> Result<ProcessInfo, Error> {
        match self.object(self.require(handle, access::QUERY)?)? {
            ObjectKind::Process(p) => Ok(ProcessInfo {
                id: p.id,
                parent: p.parent,
                terminated: p.terminated,
                exit_status: p.exit_status,
                thread_count: p.threads.len(),
            }),
            _ => Err(Error::WrongType),
        }
    }

    /// Returns thread state after checking query access.
    pub fn thread_info(&self, handle: Handle) -> Result<ThreadInfo, Error> {
        match self.object(self.require(handle, access::QUERY)?)? {
            ObjectKind::Thread(t) => Ok(ThreadInfo {
                id: t.id,
                process: t.process,
                terminated: t.terminated,
                exit_status: t.exit_status,
            }),
            _ => Err(Error::WrongType),
        }
    }

    /// Returns event state after checking query access.
    pub fn event_info(&self, handle: Handle) -> Result<EventInfo, Error> {
        match self.object(self.require(handle, access::QUERY)?)? {
            ObjectKind::Event(e) => Ok(EventInfo {
                kind: e.kind,
                signaled: e.signaled,
            }),
            _ => Err(Error::WrongType),
        }
    }

    /// Sets an event and returns its previous signal state.
    pub fn set_event(&mut self, handle: Handle) -> Result<bool, Error> {
        let object = self.require(handle, access::MODIFY_STATE)?;
        match self.object_mut(object)? {
            ObjectKind::Event(e) => Ok(core::mem::replace(&mut e.signaled, true)),
            _ => Err(Error::WrongType),
        }
    }

    /// Resets an event and returns its previous signal state.
    pub fn reset_event(&mut self, handle: Handle) -> Result<bool, Error> {
        let object = self.require(handle, access::MODIFY_STATE)?;
        match self.object_mut(object)? {
            ObjectKind::Event(e) => Ok(core::mem::replace(&mut e.signaled, false)),
            _ => Err(Error::WrongType),
        }
    }

    /// Performs an immediate wait. A non-signaled object returns `false`; the
    /// architecture scheduler can park the caller and retry after a signal.
    pub fn wait(&mut self, handle: Handle) -> Result<bool, Error> {
        let object = self.require(handle, access::SYNCHRONIZE)?;
        match self.object_mut(object)? {
            ObjectKind::Event(e) => {
                if !e.signaled {
                    return Ok(false);
                }
                if e.kind == EventKind::Synchronization {
                    e.signaled = false;
                }
                Ok(true)
            }
            ObjectKind::Process(p) => Ok(p.terminated),
            ObjectKind::Thread(t) => Ok(t.terminated),
            ObjectKind::Job(j) => {
                let j = *j;
                Ok(self.ps.job_info(j).map_or(true, |i| i.active.is_empty()))
            }
        }
    }

    /// Terminates a process and all its attached threads.
    pub fn terminate_process(&mut self, handle: Handle, status: u32) -> Result<(), Error> {
        let object = self.require(handle, access::TERMINATE)?;
        self.terminate_process_object(object, status)
    }

    fn find_ref(&self, f: impl Fn(&ObjectKind) -> bool) -> Option<ObjectRef> {
        self.objects.iter().enumerate().find_map(|(i, slot)| {
            f(slot.object.as_ref()?).then_some(ObjectRef { index: i as u32, generation: slot.generation })
        })
    }

    fn terminate_process_id(&mut self, id: ProcessId, status: u32) {
        if let Some(o) = self.find_ref(|k| matches!(k, ObjectKind::Process(p) if p.id == id)) {
            let _ = self.terminate_process_object(o, status);
        }
    }

    fn terminate_process_object(&mut self, object: ObjectRef, status: u32) -> Result<(), Error> {
        let (pid, ids) = match self.object_mut(object)? {
            ObjectKind::Process(p) => {
                if p.terminated {
                    return Err(Error::AlreadyTerminated);
                }
                p.terminated = true;
                p.exit_status = status;
                (p.id, p.threads.clone())
            }
            _ => return Err(Error::WrongType),
        };
        for id in ids {
            self.mark_thread_dead(id, status);
        }
        self.ps.process_exited(pid);
        Ok(())
    }

    fn mark_thread_dead(&mut self, id: ThreadId, status: u32) -> bool {
        let Some(o) = self.find_ref(|k| matches!(k, ObjectKind::Thread(t) if t.id == id)) else { return false };
        if let Ok(ObjectKind::Thread(t)) = self.object_mut(o) {
            if t.terminated {
                return false;
            }
            t.terminated = true;
            t.exit_status = status;
        }
        self.ps.thread_exited(id);
        true
    }

    /// Terminates one thread.
    pub fn terminate_thread(&mut self, handle: Handle, status: u32) -> Result<(), Error> {
        let object = self.require(handle, access::TERMINATE)?;
        let id = match self.object(object)? {
            ObjectKind::Thread(t) if t.terminated => return Err(Error::AlreadyTerminated),
            ObjectKind::Thread(t) => t.id,
            _ => return Err(Error::WrongType),
        };
        self.mark_thread_dead(id, status);
        Ok(())
    }

    /// `CreateJobObject`.
    pub fn create_job(&mut self, limits: ps::JobLimits) -> Result<Handle, Error> {
        let j = self.ps.create_job(limits)?;
        let o = self.alloc_object(ObjectKind::Job(j), None);
        Ok(self.add_object_handle(o))
    }

    fn job_id(&self, handle: Handle, rights: u32) -> Result<ps::JobId, Error> {
        match self.object(self.require(handle, rights)?)? {
            ObjectKind::Job(j) => Ok(*j),
            _ => Err(Error::WrongType),
        }
    }

    /// `AssignProcessToJobObject`.
    pub fn assign_to_job(&mut self, job: Handle, process: Handle) -> Result<(), Error> {
        let j = self.job_id(job, access::MODIFY_STATE)?;
        let pid = self.live_process(process, access::MODIFY_STATE)?;
        Ok(self.ps.assign(j, pid)?)
    }

    /// `TerminateJobObject`: returns how many processes were terminated.
    pub fn terminate_job(&mut self, job: Handle, status: u32) -> Result<usize, Error> {
        let pids = self.ps.terminate_job(self.job_id(job, access::TERMINATE)?)?;
        for &p in &pids {
            self.terminate_process_id(p, status);
        }
        Ok(pids.len())
    }

    /// Job accounting.
    pub fn job_info(&self, job: Handle) -> Result<ps::JobInfo, Error> {
        Ok(self.ps.job_info(self.job_id(job, access::QUERY)?)?)
    }

    /// Process id behind a handle, if still running.
    pub fn live_process(&self, handle: Handle, rights: u32) -> Result<ProcessId, Error> {
        match self.object(self.require(handle, rights)?)? {
            ObjectKind::Process(p) if p.terminated => Err(Error::AlreadyTerminated),
            ObjectKind::Process(p) => Ok(p.id),
            _ => Err(Error::WrongType),
        }
    }

    /// Thread id behind a handle, if still running.
    pub fn live_thread(&self, handle: Handle, rights: u32) -> Result<ThreadId, Error> {
        match self.object(self.require(handle, rights)?)? {
            ObjectKind::Thread(t) if t.terminated => Err(Error::AlreadyTerminated),
            ObjectKind::Thread(t) => Ok(t.id),
            _ => Err(Error::WrongType),
        }
    }

    /// `DeleteFiber`; terminates the thread that deleted its running fiber.
    pub fn delete_fiber(&mut self, fiber: ps::FiberId) -> Result<Option<ThreadId>, Error> {
        let t = self.ps.delete_fiber(fiber)?;
        if let Some(t) = t {
            self.mark_thread_dead(t, 0);
        }
        Ok(t)
    }
}

fn strip_prefix_ascii_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn normalize_named_path(name: &str) -> Result<(String, ObjectNamespace), Error> {
    if name.is_empty() {
        return Err(Error::InvalidName);
    }
    let mut path = String::new();
    for character in name.chars() {
        path.push(if character == '/' { '\\' } else { character });
    }
    let rest = strip_prefix_ascii_ci(&path, "\\BaseNamedObjects\\")
        .or_else(|| strip_prefix_ascii_ci(&path, "Global\\"))
        .or_else(|| strip_prefix_ascii_ci(&path, "Local\\"))
        .ok_or(Error::InvalidName)?;
    if rest.is_empty() || rest.len() > 512 {
        return Err(Error::InvalidName);
    }
    let mut canonical = String::new();
    for component in rest.split('\\') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.chars().any(|character| character == '\0')
        {
            return Err(Error::InvalidName);
        }
        if !canonical.is_empty() {
            canonical.push('\\');
        }
        canonical.push_str(component);
    }
    Ok((canonical, ObjectNamespace::BaseNamedObjects))
}

/// Stable syscall numbers owned by the LoOS ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SyscallNumber {
    /// Close an object handle.
    Close = 0,
    /// Duplicate an object handle.
    Duplicate = 1,
    /// Create an event; arg0 is kind (0 notification, 1 synchronization), arg1 initial.
    CreateEvent = 2,
    /// Set an event.
    SetEvent = 3,
    /// Reset an event.
    ResetEvent = 4,
    /// Wait for one object; arg1 is a timeout hint for the scheduler.
    WaitSingle = 5,
    /// Create a process; arg0 is parent handle or zero.
    CreateProcess = 6,
    /// Create a thread; arg0 is process handle.
    CreateThread = 7,
    /// Terminate a process; arg0 handle, arg1 status.
    TerminateProcess = 8,
    /// Terminate a thread; arg0 handle, arg1 status.
    TerminateThread = 9,
    /// Query object kind and identifier. arg0 handle, arg1 selector (0 kind, 1 id).
    QueryObject = 10,
    /// Create a job; arg0 flags (1 kill-on-close, 2 breakaway ok), arg1 process
    /// limit, arg2 affinity, arg3 priority class 1..=6 (0 = none each).
    CreateJob = 11,
    /// Assign arg1 process to arg0 job.
    AssignJob = 12,
    /// Terminate every process in job arg0 with status arg1; returns count.
    TerminateJob = 13,
    /// Process arg0, class arg1 (1 idle .. 6 realtime).
    SetPriorityClass = 14,
    /// Thread arg0, relative priority arg1 (signed).
    SetThreadPriority = 15,
    /// Process or thread arg0, mask arg1; thread returns previous mask.
    SetAffinity = 16,
    /// Thread arg0; returns previous suspend count.
    SuspendThread = 17,
    /// Thread arg0; returns previous suspend count.
    ResumeThread = 18,
    /// Thread arg0, param arg1; returns fiber id.
    ConvertThreadToFiber = 19,
    /// Process arg0, stack arg1, start arg2, param arg3; returns fiber id.
    CreateFiber = 20,
    /// Thread arg0 switches to fiber arg1; returns the previous fiber.
    SwitchToFiber = 21,
    /// Fiber arg0; returns the thread id terminated by it, or 0.
    DeleteFiber = 22,
    /// Thread arg0.
    ConvertFiberToThread = 23,
    /// Thread arg0, arg1: 0 base priority, 1 runnable, 2 affinity, 3 current fiber.
    QuerySched = 24,
}

impl SyscallNumber {
    fn from_raw(raw: u32) -> Option<Self> {
        Some(match raw {
            0 => Self::Close,
            1 => Self::Duplicate,
            2 => Self::CreateEvent,
            3 => Self::SetEvent,
            4 => Self::ResetEvent,
            5 => Self::WaitSingle,
            6 => Self::CreateProcess,
            7 => Self::CreateThread,
            8 => Self::TerminateProcess,
            9 => Self::TerminateThread,
            10 => Self::QueryObject,
            11 => Self::CreateJob,
            12 => Self::AssignJob,
            13 => Self::TerminateJob,
            14 => Self::SetPriorityClass,
            15 => Self::SetThreadPriority,
            16 => Self::SetAffinity,
            17 => Self::SuspendThread,
            18 => Self::ResumeThread,
            19 => Self::ConvertThreadToFiber,
            20 => Self::CreateFiber,
            21 => Self::SwitchToFiber,
            22 => Self::DeleteFiber,
            23 => Self::ConvertFiberToThread,
            24 => Self::QuerySched,
            _ => return None,
        })
    }
}

/// Syscall status returned by the architecture-independent dispatcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    /// Operation completed.
    Success = 0,
    /// Raw number or argument is invalid.
    InvalidParameter = -1,
    /// Handle is stale or malformed.
    InvalidHandle = -2,
    /// Object kind does not support the operation.
    ObjectTypeMismatch = -3,
    /// Handle lacks the required right.
    AccessDenied = -4,
    /// Named/identified object is absent.
    NotFound = -5,
    /// Object was already terminated.
    AlreadyTerminated = -6,
    /// A count or quota limit was hit.
    LimitExceeded = -7,
    /// Object is in the wrong state (fiber/job membership).
    InvalidState = -8,
}

impl From<Error> for Status {
    fn from(error: Error) -> Self {
        match error {
            Error::InvalidHandle => Self::InvalidHandle,
            Error::WrongType => Self::ObjectTypeMismatch,
            Error::AccessDenied => Self::AccessDenied,
            Error::NotFound => Self::NotFound,
            Error::AlreadyTerminated => Self::AlreadyTerminated,
            Error::InvalidName => Self::InvalidParameter,
            Error::LimitExceeded => Self::LimitExceeded,
            Error::InvalidState => Self::InvalidState,
        }
    }
}

/// Six register-sized syscall arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallArgs(pub [u64; 6]);

/// Return value from [`dispatch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallResult {
    /// Status code.
    pub status: Status,
    /// Primary result (a raw handle, boolean, or identifier).
    pub value: u64,
}

impl SyscallResult {
    const fn ok(value: u64) -> Self {
        Self {
            status: Status::Success,
            value,
        }
    }
    const fn err(status: Status) -> Self {
        Self { status, value: 0 }
    }
}

fn raw_handle(value: u64) -> Result<Handle, Status> {
    Handle::from_raw(value).ok_or(Status::InvalidHandle)
}

fn syscall_result<T>(result: Result<T, Error>) -> Result<T, Status> {
    result.map_err(Status::from)
}

/// Dispatches the first object/process/thread syscalls.
pub fn dispatch(manager: &mut ObjectManager, number: u32, args: SyscallArgs) -> SyscallResult {
    let number = match SyscallNumber::from_raw(number) {
        Some(value) => value,
        None => return SyscallResult::err(Status::InvalidParameter),
    };
    let result = match number {
        SyscallNumber::Close => {
            raw_handle(args.0[0]).and_then(|h| syscall_result(manager.close(h).map(|_| 0)))
        }
        SyscallNumber::Duplicate => raw_handle(args.0[0]).and_then(|h| {
            syscall_result(
                manager
                    .duplicate(h, args.0[1] as u32, args.0[2] != 0)
                    .map(|v| v.raw()),
            )
        }),
        SyscallNumber::CreateEvent => Ok(manager
            .create_event(
                if args.0[0] == 0 {
                    EventKind::Notification
                } else {
                    EventKind::Synchronization
                },
                args.0[1] != 0,
            )
            .raw()),
        SyscallNumber::SetEvent => raw_handle(args.0[0])
            .and_then(|h| syscall_result(manager.set_event(h).map(|v| v as u64))),
        SyscallNumber::ResetEvent => raw_handle(args.0[0])
            .and_then(|h| syscall_result(manager.reset_event(h).map(|v| v as u64))),
        SyscallNumber::WaitSingle => {
            raw_handle(args.0[0]).and_then(|h| syscall_result(manager.wait(h).map(|v| v as u64)))
        }
        SyscallNumber::CreateProcess => {
            let parent = if args.0[0] == 0 {
                Ok(None)
            } else {
                raw_handle(args.0[0])
                    .and_then(|h| syscall_result(manager.process_info(h).map(|p| Some(p.id))))
            };
            parent.and_then(|p| syscall_result(manager.create_process_ex(p, args.0[1] & 1 != 0).map(|v| v.0.raw())))
        }
        SyscallNumber::CreateThread => raw_handle(args.0[0])
            .and_then(|h| syscall_result(manager.create_thread_ex(h, args.0[1] & 1 != 0).map(|v| v.0.raw()))),
        SyscallNumber::TerminateProcess => raw_handle(args.0[0]).and_then(|h| {
            syscall_result(manager.terminate_process(h, args.0[1] as u32).map(|_| 0))
        }),
        SyscallNumber::TerminateThread => raw_handle(args.0[0])
            .and_then(|h| syscall_result(manager.terminate_thread(h, args.0[1] as u32).map(|_| 0))),
        SyscallNumber::QueryObject => raw_handle(args.0[0]).and_then(|h| match args.0[1] {
            0 => syscall_result(manager.object_type(h).map(|t| t as u64)),
            1 => match syscall_result(manager.object_type(h))? {
                ObjectType::Process => syscall_result(manager.process_info(h).map(|p| p.id)),
                ObjectType::Thread => syscall_result(manager.thread_info(h).map(|t| t.id)),
                ObjectType::Event => Ok(0),
                ObjectType::Job => Ok(0),
            },
            _ => Err(Status::InvalidParameter),
        }),
        n => ps_syscall(manager, n, &args.0),
    };
    match result {
        Ok(value) => SyscallResult::ok(value),
        Err(status) => SyscallResult::err(status),
    }
}

fn class_arg(v: u64) -> Result<Option<ps::PriorityClass>, Status> {
    use ps::PriorityClass::*;
    Ok(Some(match v {
        0 => return Ok(None),
        1 => Idle,
        2 => BelowNormal,
        3 => Normal,
        4 => AboveNormal,
        5 => High,
        6 => Realtime,
        _ => return Err(Status::InvalidParameter),
    }))
}

fn ps_syscall(m: &mut ObjectManager, n: SyscallNumber, a: &[u64; 6]) -> Result<u64, Status> {
    use access::{MODIFY_STATE as MS, QUERY as Q};
    let h = |i: usize| raw_handle(a[i]);
    let ps_r = |r: Result<u64, ps::PsError>| syscall_result(r.map_err(Error::from));
    match n {
        SyscallNumber::CreateJob => {
            let limits = ps::JobLimits {
                kill_on_close: a[0] & 1 != 0,
                breakaway_ok: a[0] & 2 != 0,
                active_processes: (a[1] != 0).then_some(a[1] as u32),
                affinity: (a[2] != 0).then_some(a[2]),
                priority_class: class_arg(a[3])?,
            };
            syscall_result(m.create_job(limits).map(Handle::raw))
        }
        SyscallNumber::AssignJob => syscall_result(m.assign_to_job(h(0)?, h(1)?).map(|_| 0)),
        SyscallNumber::TerminateJob => syscall_result(m.terminate_job(h(0)?, a[1] as u32).map(|n| n as u64)),
        SyscallNumber::SetPriorityClass => {
            let c = class_arg(a[1])?.ok_or(Status::InvalidParameter)?;
            let p = syscall_result(m.live_process(h(0)?, MS))?;
            ps_r(m.ps.set_priority_class(p, c).map(|_| 0))
        }
        SyscallNumber::SetThreadPriority => {
            let t = syscall_result(m.live_thread(h(0)?, MS))?;
            let rel = i8::try_from(a[1] as i64).map_err(|_| Status::InvalidParameter)?;
            ps_r(m.ps.set_thread_priority(t, rel).map(|_| 0))
        }
        SyscallNumber::SetAffinity => match syscall_result(m.object_type(h(0)?))? {
            ObjectType::Process => {
                let p = syscall_result(m.live_process(h(0)?, MS))?;
                ps_r(m.ps.set_process_affinity(p, a[1]).map(|_| 0))
            }
            ObjectType::Thread => {
                let t = syscall_result(m.live_thread(h(0)?, MS))?;
                ps_r(m.ps.set_thread_affinity(t, a[1]))
            }
            _ => Err(Status::ObjectTypeMismatch),
        },
        SyscallNumber::SuspendThread => {
            let t = syscall_result(m.live_thread(h(0)?, MS))?;
            ps_r(m.ps.suspend(t).map(u64::from))
        }
        SyscallNumber::ResumeThread => {
            let t = syscall_result(m.live_thread(h(0)?, MS))?;
            ps_r(m.ps.resume(t).map(u64::from))
        }
        SyscallNumber::ConvertThreadToFiber => {
            let t = syscall_result(m.live_thread(h(0)?, MS))?;
            ps_r(m.ps.convert_thread_to_fiber(t, a[1]))
        }
        SyscallNumber::CreateFiber => {
            let p = syscall_result(m.live_process(h(0)?, MS))?;
            ps_r(m.ps.create_fiber(p, a[1], a[2], a[3]))
        }
        SyscallNumber::SwitchToFiber => {
            let t = syscall_result(m.live_thread(h(0)?, MS))?;
            ps_r(m.ps.switch_to_fiber(t, a[1]))
        }
        SyscallNumber::DeleteFiber => syscall_result(m.delete_fiber(a[0]).map(|t| t.unwrap_or(0))),
        SyscallNumber::ConvertFiberToThread => {
            let t = syscall_result(m.live_thread(h(0)?, MS))?;
            ps_r(m.ps.convert_fiber_to_thread(t).map(|_| 0))
        }
        SyscallNumber::QuerySched => {
            let t = syscall_result(m.live_thread(h(0)?, Q))?;
            ps_r(match a[1] {
                0 => m.ps.base_priority(t).map(|p| p as u64),
                1 => m.ps.runnable(t).map(u64::from),
                2 => m.ps.thread_affinity(t),
                3 => m.ps.current_fiber(t).map(|f| f.unwrap_or(0)),
                _ => return Err(Status::InvalidParameter),
            })
        }
        _ => Err(Status::InvalidParameter),
    }
}
