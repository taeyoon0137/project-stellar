//! DocumentThread event loop (XII.1.1).
//!
//! 아키텍처 §9.2: Document Thread는 **유일한 문서 작성자**이며 command를 순차 적용합니다. 문서
//! 모델에 대한 다중 thread 동시 쓰기는 금지합니다. 이 모듈은 그 규칙을 타입으로 강제합니다.
//!
//! - 열린 문서 집합([`OpenDocumentSet`])은 document thread 안에서 만들어지고 그 thread를
//!   벗어나지 않습니다. [`DocumentThread`]와 [`DocumentSender`]는 문서에 대한 어떤 참조도
//!   돌려주지 않습니다.
//! - 다른 thread는 job(`FnOnce(&mut DocumentContext) -> T`)을 보내기만 합니다. job은 document
//!   thread에서 접수 순서대로 하나씩 실행되고, 결과 `T`(소유 값)만 [`JobTicket`]으로 돌아옵니다.
//! - [`DocumentContext`]와 [`DocumentMut`]는 `!Send`·`!Sync`입니다. job 안에서 다른 thread
//!   (`std::thread::spawn`, `std::thread::scope` 포함)로 넘겨 변경할 수 없고, job closure의
//!   higher-ranked lifetime 때문에 job 밖으로 빼돌릴 수도 없습니다(compile_fail doctest로 고정).
//!   가변 [`Document`] 참조(`&mut Document`)는 공개 API로 내보내지 않습니다.
//! - document thread는 [`ThreadRole::Document`]로 bind되어 worker 전용 작업(XI.7.1)을 거부하고,
//!   document thread에서의 blocking 대기([`JobTicket::wait`])도 거부합니다(자기 대기 deadlock 방지).
//!
//! # 종료
//!
//! [`DocumentThread::shutdown`]은 새 job 접수를 닫고, 이미 접수된 job을 모두 실행한 뒤 thread를
//! join합니다. thread별 전체 종료 순서는 XII.1.6 소관입니다.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use stellar_core_types::ids::{RuntimeEntityId, StableId};
use stellar_core_types::revision::DocumentRevision;
use stellar_document::node_columns::NodeFields;
use stellar_document::open_set::{OpenDocumentHandle, OpenDocumentSet, OpenOutcome};
use stellar_document::snapshot::{Document, DocumentSnapshotView};
use stellar_renderer_core::thread_role::{
    bind_thread_role, current_thread_role, ThreadRole, ThreadRoleError,
};

/// document thread 이름(디버거·trace 표시용).
pub const DOCUMENT_THREAD_NAME: &str = "stellar-document";

/// 현재 thread가 document thread인지 여부.
pub fn on_document_thread() -> bool {
    current_thread_role() == Some(ThreadRole::Document)
}

// ---------------------------------------------------------------------------
// DocumentContext / DocumentMut
// ---------------------------------------------------------------------------

/// document thread만 소유하는 문서 상태. job은 `&mut DocumentContext`로만 문서를 바꿉니다.
///
/// 외부에서 만들 수 없고(생성자 비공개), `!Send`·`!Sync`라 document thread 밖으로 나갈 수
/// 없습니다.
///
/// # 사용
///
/// ```
/// use stellar_core_types::ids::StableId;
/// use stellar_runtime::document_thread::DocumentThread;
///
/// let thread = DocumentThread::spawn().unwrap();
/// let ticket = thread
///     .sender()
///     .submit(|ctx| {
///         let handle = ctx.open_document(StableId::new_v7(1, 0, 1)).handle();
///         let doc = ctx.document_mut(handle).unwrap();
///         doc.revision()
///     })
///     .unwrap();
/// assert_eq!(ticket.wait().unwrap().0, 0);
/// thread.shutdown().unwrap();
/// ```
///
/// # 다른 thread에서의 직접 변경은 컴파일 실패
///
/// job 안에서 scoped thread로 context를 넘겨 변경할 수 없습니다(`DocumentContext: !Send`).
///
/// ```compile_fail
/// use stellar_core_types::ids::StableId;
/// use stellar_runtime::document_thread::DocumentThread;
///
/// let thread = DocumentThread::spawn().unwrap();
/// let _ = thread.sender().submit(|ctx| {
///     std::thread::scope(|s| {
///         s.spawn(|| {
///             ctx.open_document(StableId::new_v7(1, 0, 1));
///         });
///     });
/// });
/// ```
///
/// 문서 변경 view([`DocumentMut`])도 다른 thread로 옮길 수 없습니다.
///
/// ```compile_fail
/// use stellar_core_types::ids::StableId;
/// use stellar_runtime::document_thread::DocumentThread;
///
/// let thread = DocumentThread::spawn().unwrap();
/// let _ = thread.sender().submit(|ctx| {
///     let handle = ctx.open_document(StableId::new_v7(1, 0, 1)).handle();
///     let doc = ctx.document_mut(handle).unwrap();
///     std::thread::scope(|s| {
///         s.spawn(move || {
///             let _moved = doc;
///         });
///     });
/// });
/// ```
///
/// context를 job 밖(`'static`)으로 빼돌릴 수 없습니다.
///
/// ```compile_fail
/// use stellar_runtime::document_thread::{DocumentContext, DocumentThread};
///
/// fn keep(_: &'static mut DocumentContext) {}
/// let thread = DocumentThread::spawn().unwrap();
/// let _ = thread.sender().submit(|ctx| keep(ctx));
/// ```
///
/// `DocumentContext`와 `DocumentMut`는 `Send`가 아닙니다.
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<stellar_runtime::document_thread::DocumentContext>();
/// ```
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<stellar_runtime::document_thread::DocumentMut<'static>>();
/// ```
///
/// document thread 밖에서 문서 상태를 따로 만들 수 없습니다.
///
/// ```compile_fail
/// let _ctx = stellar_runtime::document_thread::DocumentContext::new();
/// ```
#[derive(Debug)]
pub struct DocumentContext {
    documents: OpenDocumentSet,
    revisions: HashMap<OpenDocumentHandle, DocumentRevision>,
    _not_send: PhantomData<*const ()>,
}

impl DocumentContext {
    fn new() -> DocumentContext {
        DocumentContext {
            documents: OpenDocumentSet::new(),
            revisions: HashMap::new(),
            _not_send: PhantomData,
        }
    }

    /// 문서를 엽니다. 이미 열려 있으면 기존 handle을 반환합니다(VI.1.4). 새로 열린 문서의
    /// revision은 0입니다.
    pub fn open_document(&mut self, document_id: StableId) -> OpenOutcome {
        let outcome = self.documents.open(document_id);
        if let OpenOutcome::NewlyOpened(handle) = &outcome {
            self.revisions.insert(*handle, DocumentRevision(0));
        }
        outcome
    }

    /// 문서를 닫습니다. 이후 그 handle은 무효입니다. 성공 여부를 반환합니다.
    pub fn close_document(&mut self, handle: OpenDocumentHandle) -> bool {
        let closed = self.documents.close(handle);
        if closed {
            self.revisions.remove(&handle);
        }
        closed
    }

    /// 해당 문서가 열려 있는지.
    pub fn is_open(&self, document_id: StableId) -> bool {
        self.documents.is_open(document_id)
    }

    /// 열린 문서 수.
    pub fn open_count(&self) -> usize {
        self.documents.len()
    }

    /// handle의 문서 StableId. stale handle이면 `None`.
    pub fn document_id(&self, handle: OpenDocumentHandle) -> Option<StableId> {
        self.documents.document_id(handle)
    }

    /// handle의 읽기 전용 view. stale handle이면 `None`.
    pub fn snapshot(&self, handle: OpenDocumentHandle) -> Option<DocumentSnapshotView<'_>> {
        self.documents.document(handle).map(Document::snapshot)
    }

    /// handle의 현재 revision. stale handle이면 `None`.
    pub fn revision(&self, handle: OpenDocumentHandle) -> Option<DocumentRevision> {
        self.revisions.get(&handle).copied()
    }

    /// handle의 문서 변경 view. stale handle이면 `None`.
    pub fn document_mut(&mut self, handle: OpenDocumentHandle) -> Option<DocumentMut<'_>> {
        let document = self.documents.document_mut(handle)?;
        let revision = self.revisions.get_mut(&handle)?;
        Some(DocumentMut {
            document,
            revision,
            _not_send: PhantomData,
        })
    }
}

/// 열린 문서 하나의 변경 view. 성공한 변경마다 문서 revision을 1 올립니다.
///
/// `!Send`·`!Sync`라 document thread 밖으로 옮길 수 없습니다([`DocumentContext`] 문서 참조).
#[derive(Debug)]
pub struct DocumentMut<'a> {
    document: &'a mut Document,
    revision: &'a mut DocumentRevision,
    _not_send: PhantomData<*const ()>,
}

impl DocumentMut<'_> {
    /// 노드를 추가하고 핸들을 반환합니다.
    pub fn insert_node(&mut self, fields: NodeFields) -> RuntimeEntityId {
        let id = self.document.insert_node(fields);
        self.bump();
        id
    }

    /// 노드를 제거합니다. stale 핸들이면 `false`이고 revision은 그대로입니다.
    pub fn remove_node(&mut self, id: RuntimeEntityId) -> bool {
        let removed = self.document.remove_node(id);
        if removed {
            self.bump();
        }
        removed
    }

    /// 부모를 갱신합니다. stale 핸들이면 `false`이고 revision은 그대로입니다.
    pub fn set_parent(&mut self, id: RuntimeEntityId, parent: Option<RuntimeEntityId>) -> bool {
        let updated = self.document.set_parent(id, parent);
        if updated {
            self.bump();
        }
        updated
    }

    /// 읽기 전용 view.
    pub fn snapshot(&self) -> DocumentSnapshotView<'_> {
        self.document.snapshot()
    }

    /// 현재 revision.
    pub fn revision(&self) -> DocumentRevision {
        *self.revision
    }

    fn bump(&mut self) {
        self.revision.0 = self.revision.0.saturating_add(1);
    }
}

// ---------------------------------------------------------------------------
// job·ticket
// ---------------------------------------------------------------------------

/// job이 결과를 돌려주지 못한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobError {
    /// document thread가 job을 끝내기 전에 사라졌습니다(실행 전 비정상 종료, job panic).
    Lost,
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobError::Lost => write!(f, "document thread가 job을 끝내기 전에 종료됐습니다"),
        }
    }
}

impl std::error::Error for JobError {}

/// ticket 조회 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketError {
    /// job이 결과를 돌려주지 못했습니다.
    Job(JobError),
    /// 결과를 이미 꺼냈습니다.
    AlreadyTaken,
    /// 현재 thread 역할에서는 blocking 대기가 금지됩니다.
    BlockingNotAllowed {
        /// 현재 thread 역할.
        role: ThreadRole,
    },
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TicketError::Job(error) => write!(f, "job 실패: {error}"),
            TicketError::AlreadyTaken => write!(f, "job 결과를 이미 꺼냈습니다"),
            TicketError::BlockingNotAllowed { role } => {
                write!(
                    f,
                    "{role:?} thread에서는 job 결과를 blocking으로 기다릴 수 없습니다"
                )
            }
        }
    }
}

impl std::error::Error for TicketError {}

type JobReply<T> = Result<T, JobError>;

/// 타입을 지운 job. document thread에서 한 번 실행됩니다.
trait ErasedJob: Send {
    fn run(self: Box<Self>, context: &mut DocumentContext);
}

struct TypedJob<F, T> {
    job: F,
    reply: Sender<JobReply<T>>,
}

impl<F, T> ErasedJob for TypedJob<F, T>
where
    F: FnOnce(&mut DocumentContext) -> T + Send,
    T: Send,
{
    fn run(self: Box<Self>, context: &mut DocumentContext) {
        let TypedJob { job, reply } = *self;
        let value = job(context);
        // ticket을 버린 호출자에게는 결과를 전달하지 않습니다(정상 경로).
        let _ = reply.send(Ok(value));
    }
}

/// blocking 대기가 허용되는 thread인지 확인합니다.
fn ensure_blocking_allowed() -> Result<(), TicketError> {
    match current_thread_role() {
        // document thread가 자기 queue의 job을 기다리면 deadlock입니다.
        Some(role @ ThreadRole::Document) => Err(TicketError::BlockingNotAllowed { role }),
        _ => Ok(()),
    }
}

/// 접수된 job의 결과 handle. 결과는 한 번만 꺼낼 수 있습니다.
pub struct JobTicket<T> {
    reply: Receiver<JobReply<T>>,
    taken: bool,
}

impl<T> fmt::Debug for JobTicket<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobTicket")
            .field("taken", &self.taken)
            .finish()
    }
}

impl<T> JobTicket<T> {
    fn new(reply: Receiver<JobReply<T>>) -> JobTicket<T> {
        JobTicket {
            reply,
            taken: false,
        }
    }

    /// 결과를 이미 꺼냈는지(또는 실패가 확정됐는지).
    pub fn is_taken(&self) -> bool {
        self.taken
    }

    /// 기다리지 않고 결과를 확인합니다. 아직 실행 전이면 `Ok(None)`입니다.
    pub fn try_take(&mut self) -> Result<Option<T>, TicketError> {
        if self.taken {
            return Err(TicketError::AlreadyTaken);
        }
        match self.reply.try_recv() {
            Ok(reply) => self.finish(reply).map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => self.finish(Err(JobError::Lost)).map(Some),
        }
    }

    /// 최대 `timeout`만큼 결과를 기다립니다. 시간이 지나면 `Ok(None)`입니다.
    ///
    /// document thread에서는 거부합니다([`TicketError::BlockingNotAllowed`]).
    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<T>, TicketError> {
        if self.taken {
            return Err(TicketError::AlreadyTaken);
        }
        ensure_blocking_allowed()?;
        match self.reply.recv_timeout(timeout) {
            Ok(reply) => self.finish(reply).map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => self.finish(Err(JobError::Lost)).map(Some),
        }
    }

    /// 결과가 나올 때까지 기다립니다.
    ///
    /// document thread에서는 거부합니다([`TicketError::BlockingNotAllowed`]).
    pub fn wait(mut self) -> Result<T, TicketError> {
        if self.taken {
            return Err(TicketError::AlreadyTaken);
        }
        ensure_blocking_allowed()?;
        match self.reply.recv() {
            Ok(reply) => self.finish(reply),
            Err(_) => self.finish(Err(JobError::Lost)),
        }
    }

    fn finish(&mut self, reply: JobReply<T>) -> Result<T, TicketError> {
        self.taken = true;
        reply.map_err(TicketError::Job)
    }
}

// ---------------------------------------------------------------------------
// inbox
// ---------------------------------------------------------------------------

/// UI·다른 thread → document thread job 수신함(FIFO).
///
/// 임계 구역은 push/pop 한 번(O(1))뿐이며 job 실행·drop은 항상 lock 밖에서 합니다.
struct Inbox {
    state: Mutex<InboxState>,
    ready: Condvar,
}

struct InboxState {
    jobs: VecDeque<Box<dyn ErasedJob>>,
    closed: bool,
}

impl Inbox {
    fn new() -> Inbox {
        Inbox {
            state: Mutex::new(InboxState {
                jobs: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, InboxState> {
        // 임계 구역에서 panic하는 코드가 없으므로 poison 상태도 일관됩니다.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// job을 넣습니다. 닫혀 있으면 job을 돌려줍니다(호출자가 lock 밖에서 버립니다).
    fn push(&self, job: Box<dyn ErasedJob>) -> Result<(), Box<dyn ErasedJob>> {
        let mut state = self.lock();
        if state.closed {
            return Err(job);
        }
        state.jobs.push_back(job);
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    /// 다음 job을 꺼냅니다. 비어 있으면 기다리고, 닫힌 뒤 비면 `None`입니다.
    fn pop_blocking(&self) -> Option<Box<dyn ErasedJob>> {
        let mut state = self.lock();
        loop {
            if let Some(job) = state.jobs.pop_front() {
                return Some(job);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// 새 접수를 닫습니다. 이미 접수된 job은 그대로 둡니다.
    fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        drop(state);
        self.ready.notify_all();
    }

    /// 닫고 남은 job을 꺼냅니다(비정상 종료 경로).
    fn close_and_take(&self) -> VecDeque<Box<dyn ErasedJob>> {
        let mut state = self.lock();
        state.closed = true;
        let remaining = std::mem::take(&mut state.jobs);
        drop(state);
        self.ready.notify_all();
        remaining
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

/// document thread가 어떤 경로로 끝나든(panic 포함) 수신함을 닫고 남은 job을 버립니다.
struct CloseOnExit {
    inbox: Arc<Inbox>,
}

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        // 남은 job은 lock 밖에서 버립니다. 각 ticket은 `JobError::Lost`가 됩니다.
        let remaining = self.inbox.close_and_take();
        drop(remaining);
    }
}

// ---------------------------------------------------------------------------
// sender·thread
// ---------------------------------------------------------------------------

/// job 접수 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// document thread가 종료 중이거나 종료됐습니다.
    Stopped,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubmitError::Stopped => write!(f, "document thread가 job 접수를 닫았습니다"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// 다른 thread가 document thread에 job을 보내는 handle. 복제해 여러 thread에서 쓸 수 있습니다.
///
/// 문서 참조를 돌려주는 API가 없습니다. 모든 변경은 [`DocumentSender::submit`]한 job 안에서
/// document thread가 수행합니다.
#[derive(Clone)]
pub struct DocumentSender {
    inbox: Arc<Inbox>,
}

impl fmt::Debug for DocumentSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentSender")
            .field("stopped", &self.is_stopped())
            .finish()
    }
}

impl DocumentSender {
    /// job을 접수합니다. job 실행을 기다리지 않고 바로 반환합니다.
    pub fn submit<F, T>(&self, job: F) -> Result<JobTicket<T>, SubmitError>
    where
        F: FnOnce(&mut DocumentContext) -> T + Send + 'static,
        T: Send + 'static,
    {
        let (reply, receiver) = mpsc::channel();
        let job: Box<dyn ErasedJob> = Box::new(TypedJob { job, reply });
        match self.inbox.push(job) {
            Ok(()) => Ok(JobTicket::new(receiver)),
            Err(rejected) => {
                drop(rejected);
                Err(SubmitError::Stopped)
            }
        }
    }

    /// job 접수가 닫혔는지.
    pub fn is_stopped(&self) -> bool {
        self.inbox.is_closed()
    }
}

/// event loop 종료 통계.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocumentLoopStats {
    /// 실행한 job 수.
    pub jobs_run: u64,
}

/// document thread 생성·종료 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentThreadError {
    /// 운영체제 thread를 만들지 못했습니다.
    Spawn {
        /// 원인 종류.
        kind: io::ErrorKind,
    },
    /// thread 역할을 bind하지 못했습니다.
    RoleBind(ThreadRoleError),
    /// document thread 자신은 자기를 종료·join할 수 없습니다(deadlock).
    ShutdownFromDocumentThread,
    /// document thread가 panic으로 끝났습니다.
    Panicked,
}

impl fmt::Display for DocumentThreadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentThreadError::Spawn { kind } => {
                write!(f, "document thread를 만들지 못했습니다: {kind}")
            }
            DocumentThreadError::RoleBind(error) => {
                write!(f, "document thread 역할 bind 실패: {error}")
            }
            DocumentThreadError::ShutdownFromDocumentThread => {
                write!(f, "document thread에서 자기 자신을 종료할 수 없습니다")
            }
            DocumentThreadError::Panicked => write!(f, "document thread가 panic으로 끝났습니다"),
        }
    }
}

impl std::error::Error for DocumentThreadError {}

/// 유일한 문서 작성자 thread와 그 event loop.
///
/// drop하면 [`DocumentThread::shutdown`]과 같이 접수를 닫고 남은 job을 처리한 뒤 join합니다.
pub struct DocumentThread {
    inbox: Arc<Inbox>,
    thread_id: ThreadId,
    join: Option<JoinHandle<Result<DocumentLoopStats, DocumentThreadError>>>,
}

impl fmt::Debug for DocumentThread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentThread")
            .field("thread_id", &self.thread_id)
            .field("running", &self.join.is_some())
            .finish()
    }
}

impl DocumentThread {
    /// document thread를 시작합니다.
    pub fn spawn() -> Result<DocumentThread, DocumentThreadError> {
        let inbox = Arc::new(Inbox::new());
        let loop_inbox = Arc::clone(&inbox);
        let join = thread::Builder::new()
            .name(DOCUMENT_THREAD_NAME.to_string())
            .spawn(move || run_loop(loop_inbox))
            .map_err(|error| DocumentThreadError::Spawn { kind: error.kind() })?;
        let thread_id = join.thread().id();
        Ok(DocumentThread {
            inbox,
            thread_id,
            join: Some(join),
        })
    }

    /// job 접수 handle.
    pub fn sender(&self) -> DocumentSender {
        DocumentSender {
            inbox: Arc::clone(&self.inbox),
        }
    }

    /// document thread의 id.
    pub fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    /// 접수를 닫고, 이미 접수된 job을 모두 실행한 뒤 thread를 join합니다.
    ///
    /// document thread 자신(job 안)에서 호출하면 접수만 닫고
    /// [`DocumentThreadError::ShutdownFromDocumentThread`]를 반환합니다.
    pub fn shutdown(mut self) -> Result<DocumentLoopStats, DocumentThreadError> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> Result<DocumentLoopStats, DocumentThreadError> {
        self.inbox.close();
        let Some(join) = self.join.take() else {
            return Ok(DocumentLoopStats::default());
        };
        if thread::current().id() == self.thread_id {
            // 자기 자신을 join하면 deadlock입니다. handle을 버려 thread를 detach하고, thread는
            // 남은 job을 처리한 뒤 스스로 끝납니다.
            drop(join);
            return Err(DocumentThreadError::ShutdownFromDocumentThread);
        }
        match join.join() {
            Ok(result) => result,
            Err(_) => Err(DocumentThreadError::Panicked),
        }
    }
}

impl Drop for DocumentThread {
    fn drop(&mut self) {
        if self.join.is_some() {
            let _ = self.stop_and_join();
        }
    }
}

/// document thread event loop: job을 접수 순서대로 하나씩 실행합니다.
fn run_loop(inbox: Arc<Inbox>) -> Result<DocumentLoopStats, DocumentThreadError> {
    let _close_on_exit = CloseOnExit {
        inbox: Arc::clone(&inbox),
    };
    let _role = bind_thread_role(ThreadRole::Document).map_err(DocumentThreadError::RoleBind)?;
    let mut context = DocumentContext::new();
    let mut stats = DocumentLoopStats::default();
    while let Some(job) = inbox.pop_blocking() {
        job.run(&mut context);
        stats.jobs_run = stats.jobs_run.saturating_add(1);
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::node_kind::NodeKind;

    fn stable(seed: u64) -> StableId {
        StableId::new_v7(seed, 0x0abc, seed)
    }

    fn fields(seed: u64) -> NodeFields {
        NodeFields {
            stable_id: stable(seed),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
        }
    }

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    /// 조건이 참이 될 때까지 최대 5초 기다립니다.
    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::yield_now();
        }
        condition()
    }

    /// document thread를 막아 두는 job과 그 해제 handle.
    fn gate(sender: &DocumentSender) -> (JobTicket<()>, Sender<()>) {
        let (release, wait) = mpsc::channel::<()>();
        let ticket = sender
            .submit(move |_ctx| {
                let _ = wait.recv();
            })
            .unwrap();
        (ticket, release)
    }

    #[test]
    fn only_handles_cross_threads() {
        assert_send::<DocumentSender>();
        assert_sync::<DocumentSender>();
        assert_send::<DocumentThread>();
        assert_send::<JobTicket<u64>>();
    }

    #[test]
    fn jobs_run_only_on_the_document_thread() {
        // 검증 기준(XII.1.1): 문서 접근은 document thread에서만 일어납니다.
        let doc = DocumentThread::spawn().unwrap();
        let caller = thread::current().id();
        assert!(!on_document_thread());
        let (id, role, name, on_doc) = doc
            .sender()
            .submit(|_ctx| {
                let current = thread::current();
                (
                    current.id(),
                    current_thread_role(),
                    current.name().map(str::to_owned),
                    on_document_thread(),
                )
            })
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(id, doc.thread_id());
        assert_ne!(id, caller);
        assert_eq!(role, Some(ThreadRole::Document));
        assert_eq!(name.as_deref(), Some(DOCUMENT_THREAD_NAME));
        assert!(on_doc);
        assert_eq!(doc.shutdown().unwrap().jobs_run, 1);
    }

    #[test]
    fn concurrent_senders_mutate_only_through_the_single_writer() {
        // 4개 thread가 동시에 job을 보내도 변경은 document thread 하나가 순차 적용합니다.
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let handle = sender
            .submit(|ctx| ctx.open_document(stable(1)).handle())
            .unwrap()
            .wait()
            .unwrap();
        let senders: Vec<_> = (0..4u64)
            .map(|t| {
                let sender = sender.clone();
                thread::spawn(move || {
                    let tickets: Vec<_> = (0..250u64)
                        .map(|i| {
                            sender
                                .submit(move |ctx| {
                                    let mut doc = ctx.document_mut(handle).expect("열린 문서");
                                    doc.insert_node(fields(t * 1_000 + i + 10));
                                    (t, doc.revision())
                                })
                                .unwrap()
                        })
                        .collect();
                    tickets
                        .into_iter()
                        .map(|ticket| ticket.wait().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut applied = Vec::new();
        for sender_thread in senders {
            applied.extend(sender_thread.join().unwrap());
        }
        // 같은 sender의 job은 접수 순서대로 적용됩니다.
        for t in 0..4u64 {
            let revisions: Vec<u64> = applied
                .iter()
                .filter(|(owner, _)| *owner == t)
                .map(|(_, revision)| revision.0)
                .collect();
            assert_eq!(revisions.len(), 250);
            assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
        }
        // 단일 작성자: 1000개 변경이 서로 다른 revision 1..=1000을 하나씩 받습니다.
        let mut revisions: Vec<u64> = applied.iter().map(|(_, revision)| revision.0).collect();
        revisions.sort_unstable();
        assert_eq!(revisions, (1..=1_000).collect::<Vec<u64>>());
        let (count, revision) = sender
            .submit(move |ctx| {
                (
                    ctx.snapshot(handle).map(|view| view.len()),
                    ctx.revision(handle),
                )
            })
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(count, Some(1_000));
        assert_eq!(revision, Some(DocumentRevision(1_000)));
        doc.shutdown().unwrap();
    }

    #[test]
    fn revision_counts_successful_mutations_and_follows_lifecycle() {
        let doc = DocumentThread::spawn().unwrap();
        let report = doc
            .sender()
            .submit(|ctx| {
                let mut log = Vec::new();
                let handle = ctx.open_document(stable(1)).handle();
                log.push(ctx.revision(handle));
                let (parent, child) = {
                    let mut doc = ctx.document_mut(handle).unwrap();
                    let parent = doc.insert_node(fields(2));
                    let child = doc.insert_node(fields(3));
                    (parent, child)
                };
                log.push(ctx.revision(handle));
                {
                    let mut doc = ctx.document_mut(handle).unwrap();
                    assert!(doc.set_parent(child, Some(parent)));
                    assert!(doc.remove_node(child));
                    // stale 핸들 변경은 실패하고 revision을 올리지 않습니다.
                    assert!(!doc.remove_node(child));
                    assert!(!doc.set_parent(child, None));
                    assert_eq!(doc.snapshot().len(), 1);
                }
                log.push(ctx.revision(handle));
                // 중복 open은 기존 handle과 revision을 유지합니다.
                let again = ctx.open_document(stable(1));
                assert_eq!(again, OpenOutcome::Existing(handle));
                log.push(ctx.revision(handle));
                assert!(ctx.close_document(handle));
                assert!(ctx.document_mut(handle).is_none());
                assert!(ctx.snapshot(handle).is_none());
                log.push(ctx.revision(handle));
                let reopened = ctx.open_document(stable(1)).handle();
                assert_ne!(reopened, handle);
                log.push(ctx.revision(reopened));
                (log, ctx.open_count(), ctx.is_open(stable(1)))
            })
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(
            report.0,
            vec![
                Some(DocumentRevision(0)),
                Some(DocumentRevision(2)),
                Some(DocumentRevision(4)),
                Some(DocumentRevision(4)),
                None,
                Some(DocumentRevision(0)),
            ]
        );
        assert_eq!(report.1, 1);
        assert!(report.2);
        doc.shutdown().unwrap();
    }

    #[test]
    fn shutdown_drains_accepted_jobs_then_rejects_new_ones() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let (_gate, release) = gate(&sender);
        let tickets: Vec<_> = (0..10u32)
            .map(|i| sender.submit(move |_ctx| i).unwrap())
            .collect();
        let stopper = thread::spawn(move || doc.shutdown());
        assert!(
            wait_until(|| sender.is_stopped()),
            "종료가 접수를 닫아야 함"
        );
        assert_eq!(sender.submit(|_ctx| ()).unwrap_err(), SubmitError::Stopped);
        release.send(()).unwrap();
        let stats = stopper.join().unwrap().unwrap();
        assert_eq!(stats.jobs_run, 11, "닫기 전에 접수된 job은 모두 실행");
        let values: Vec<u32> = tickets.into_iter().map(|t| t.wait().unwrap()).collect();
        assert_eq!(values, (0..10).collect::<Vec<u32>>());
    }

    #[test]
    fn try_take_does_not_block_and_result_is_taken_once() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let (_gate, release) = gate(&sender);
        let mut ticket = sender.submit(|_ctx| 5u32).unwrap();
        assert_eq!(ticket.try_take(), Ok(None), "실행 전에는 바로 None");
        assert!(!ticket.is_taken());
        release.send(()).unwrap();
        assert_eq!(ticket.wait_timeout(Duration::from_secs(5)), Ok(Some(5)));
        assert!(ticket.is_taken());
        assert_eq!(ticket.try_take(), Err(TicketError::AlreadyTaken));
        assert_eq!(
            ticket.wait_timeout(Duration::from_millis(1)),
            Err(TicketError::AlreadyTaken)
        );
        doc.shutdown().unwrap();
    }

    #[test]
    fn blocking_wait_is_refused_on_the_document_thread() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let inner_sender = sender.clone();
        let (timed, waited, polled) = sender
            .submit(move |_ctx| {
                let mut inner = inner_sender.submit(|_ctx| 7u32).unwrap();
                let timed = inner.wait_timeout(Duration::from_millis(1));
                let other = inner_sender.submit(|_ctx| 8u32).unwrap();
                let waited = other.wait();
                // 현재 job이 끝나야 inner가 실행되므로 아직 결과가 없습니다.
                let polled = inner.try_take();
                (timed, waited, polled)
            })
            .unwrap()
            .wait()
            .unwrap();
        let refused = TicketError::BlockingNotAllowed {
            role: ThreadRole::Document,
        };
        assert_eq!(timed, Err(refused));
        assert_eq!(waited, Err(refused));
        assert_eq!(polled, Ok(None));
        assert_eq!(doc.shutdown().unwrap().jobs_run, 3);
    }

    #[test]
    fn shutdown_from_the_document_thread_is_refused() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let result = sender
            .submit(move |_ctx| doc.shutdown())
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(result, Err(DocumentThreadError::ShutdownFromDocumentThread));
        assert!(sender.is_stopped());
    }

    #[test]
    fn dropping_the_thread_stops_and_joins() {
        let sender = {
            let doc = DocumentThread::spawn().unwrap();
            let sender = doc.sender();
            sender
                .submit(|ctx| ctx.open_document(stable(9)).handle())
                .unwrap();
            sender
        };
        assert!(sender.is_stopped());
        assert_eq!(sender.submit(|_ctx| ()).unwrap_err(), SubmitError::Stopped);
    }

    #[test]
    fn job_panic_stops_the_thread_and_reports_lost() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let panicking = sender
            .submit(|_ctx| -> u32 { panic!("의도된 job panic(테스트)") })
            .unwrap();
        assert_eq!(panicking.wait(), Err(TicketError::Job(JobError::Lost)));
        assert!(wait_until(|| sender.is_stopped()));
        assert_eq!(sender.submit(|_ctx| ()).unwrap_err(), SubmitError::Stopped);
        assert_eq!(doc.shutdown(), Err(DocumentThreadError::Panicked));
    }

    #[test]
    fn errors_display_in_korean() {
        assert!(SubmitError::Stopped.to_string().contains("닫았"));
        assert!(TicketError::AlreadyTaken.to_string().contains("이미"));
        assert!(TicketError::Job(JobError::Lost)
            .to_string()
            .contains("종료"));
        assert!(DocumentThreadError::Panicked.to_string().contains("panic"));
        assert!(DocumentThreadError::ShutdownFromDocumentThread
            .to_string()
            .contains("자기 자신"));
    }
}
