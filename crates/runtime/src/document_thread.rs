//! DocumentThread event loop (XII.1.1).
//!
//! 아키텍처 §9.2: Document Thread는 **유일한 문서 작성자**이며 command를 순차 적용합니다. 문서
//! 모델에 대한 다중 thread 동시 쓰기는 금지합니다. 이 모듈은 그 규칙을 타입으로 강제합니다.
//!
//! - 열린 문서 집합([`OpenDocumentSet`])은 document thread 안에서 만들어지고 그 thread를
//!   벗어나지 않습니다. [`DocumentThread`]와 [`DocumentSender`]는 문서에 대한 어떤 참조도
//!   돌려주지 않습니다.
//! - 다른 thread는 job(`FnOnce(&mut DocumentContext) -> T`)을 보내기만 합니다. job은 document
//!   thread에서 하나씩 실행되고, 결과 `T`(소유 값)만 [`JobTicket`]으로 돌아옵니다.
//! - [`DocumentContext`]와 [`DocumentMut`]는 `!Send`·`!Sync`입니다. job 안에서 다른 thread
//!   (`std::thread::spawn`, `std::thread::scope` 포함)로 넘겨 변경할 수 없고, job closure의
//!   higher-ranked lifetime 때문에 job 밖으로 빼돌릴 수도 없습니다(compile_fail doctest로 고정).
//!   가변 [`Document`] 참조(`&mut Document`)는 공개 API로 내보내지 않습니다.
//! - document thread는 [`ThreadRole::Document`]로 bind되어 worker 전용 작업(XI.7.1)을 거부하고,
//!   document thread에서의 blocking 대기([`JobTicket::wait`])도 거부합니다(자기 대기 deadlock 방지).
//!
//! ## 강제 범위의 한계
//!
//! 타입으로 강제하는 범위는 **[`DocumentThread`]가 소유한 문서**뿐입니다. `stellar-document`의
//! [`Document`]·[`OpenDocumentSet`]은 공개 생성자(`Document::new`, `OpenDocumentSet::new`)와 가변
//! 접근(`OpenDocumentSet::document_mut`)을 가지므로, `stellar-document`에 직접 의존할 수 있는
//! crate는 다른 thread에서 별도 문서를 만들고 바꿀 수 있습니다. 그런 문서는 이 runtime의 열린
//! 문서가 아니며 셸에 보이지 않습니다. 셸 경로는 crate 의존 정책(`ffi → document` 금지,
//! ADR-0013)으로만 이 thread를 거치도록 보장합니다.
//!
//! # 수신함 (XII.1.2)
//!
//! 수신함은 우선순위별 bounded lane을 가진 [`CommandQueue`]입니다. 처리 순서와 overflow 정책은
//! [`crate::command_queue`] 문서에 있습니다. UI thread가 쓰는 접수 API
//! ([`DocumentSender::try_submit`], [`DocumentSender::submit`])는 용량·job 실행을 기다리지 않고
//! 바로 판정하며, [`ThreadRole::Ui`] thread에서의 blocking 대기([`JobTicket::wait`],
//! [`JobTicket::wait_timeout`])는 거부합니다. UI thread는 [`JobTicket::try_take`]로만 결과를
//! 확인합니다.
//!
//! # 종료
//!
//! [`DocumentThread::shutdown`]은 새 job 접수를 닫고, 이미 접수된 job을 모두 실행한 뒤 thread를
//! join합니다. thread별 전체 종료 순서는 XII.1.6 소관입니다.
//!
//! [`DocumentThread::shutdown_timeout`]은 같은 순서로 종료하되 thread가 시간 안에 끝나지 않으면
//! 기다리지 않습니다. 실행 중인 job(예: 응답 없는 파일 시스템 호출)은 끊을 수 없으므로, 아직 시작하지
//! 않은 job만 버리고 thread를 분리(detach)합니다. 분리된 thread는 실행 중인 job이 끝나면 스스로
//! 끝납니다.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
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

use crate::app::AppSessions;
use crate::command_queue::{
    CommandPriority, CommandQueue, Pushed, QueueCapacity, RejectReason, SubmitOptions,
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
    /// 셸 경계용 프로젝트·문서 session 표([`crate::app`]). crate 밖에는 노출하지 않습니다.
    app: AppSessions,
    _not_send: PhantomData<*const ()>,
}

impl DocumentContext {
    fn new() -> DocumentContext {
        DocumentContext {
            documents: OpenDocumentSet::new(),
            revisions: HashMap::new(),
            app: AppSessions::default(),
            _not_send: PhantomData,
        }
    }

    /// 프로젝트·문서 session 표(읽기).
    pub(crate) fn app_sessions(&self) -> &AppSessions {
        &self.app
    }

    /// 프로젝트·문서 session 표(쓰기).
    pub(crate) fn app_sessions_mut(&mut self) -> &mut AppSessions {
        &mut self.app
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
    /// 같은 병합 key의 더 새 job으로 대체됐습니다(XII.1.2).
    Superseded,
    /// background lane 용량 초과로 밀려났습니다(XII.1.2).
    Evicted,
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobError::Lost => write!(f, "document thread가 job을 끝내기 전에 종료됐습니다"),
            JobError::Superseded => write!(f, "같은 병합 key의 더 새 job으로 대체됐습니다"),
            JobError::Evicted => write!(f, "background lane 용량 초과로 밀려났습니다"),
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

/// 타입을 지운 job. document thread에서 한 번 실행되거나, 실행 없이 버려집니다.
trait ErasedJob: Send {
    fn run(self: Box<Self>, context: &mut DocumentContext);

    /// 실행하지 않고 버립니다. ticket에 이유를 알립니다.
    fn discard(self: Box<Self>, reason: JobError);
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

    fn discard(self: Box<Self>, reason: JobError) {
        let TypedJob { job, reply } = *self;
        // closure(캡처 값 포함)를 먼저 버린 뒤 ticket에 알립니다.
        drop(job);
        let _ = reply.send(Err(reason));
    }
}

/// blocking 대기가 허용되는 thread인지 확인합니다.
fn ensure_blocking_allowed() -> Result<(), TicketError> {
    match current_thread_role() {
        // document thread가 자기 queue의 job을 기다리면 deadlock입니다.
        // UI thread는 document thread를 기다리지 않습니다(XII.1.2, blocking UI call 금지).
        Some(role @ (ThreadRole::Document | ThreadRole::Ui)) => {
            Err(TicketError::BlockingNotAllowed { role })
        }
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
    /// document·UI thread에서는 거부합니다([`TicketError::BlockingNotAllowed`]).
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
    /// document·UI thread에서는 거부합니다([`TicketError::BlockingNotAllowed`]).
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

/// UI·다른 thread → document thread job 수신함: 우선순위별 bounded lane(XII.1.2).
///
/// 임계 구역은 접수·꺼내기 한 번(O(1))뿐이며 job 실행·폐기는 항상 lock 밖에서 합니다.
type Inbox = CommandQueue<Box<dyn ErasedJob>>;

/// document thread가 어떤 경로로 끝나든(panic 포함) 수신함을 닫고 남은 job을 버립니다.
struct CloseOnExit {
    inbox: Arc<Inbox>,
}

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        // 남은 job은 lock 밖에서 버립니다. 각 ticket은 `JobError::Lost`가 됩니다.
        for job in self.inbox.close_and_drain() {
            job.discard(JobError::Lost);
        }
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
    /// 해당 우선순위 lane 용량이 찼습니다(XII.1.2). job은 접수되지 않았고 실행되지 않습니다.
    QueueFull {
        /// 해당 lane.
        priority: CommandPriority,
    },
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubmitError::Stopped => write!(f, "document thread가 job 접수를 닫았습니다"),
            SubmitError::QueueFull { priority } => {
                write!(
                    f,
                    "{} lane 용량이 차 job을 접수하지 못했습니다",
                    priority.name()
                )
            }
        }
    }
}

impl std::error::Error for SubmitError {}

/// 접수 판정(XII.1.2 overflow 정책).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// lane 끝에 접수했습니다.
    Enqueued,
    /// 같은 병합 key의 마지막 job을 대체했습니다(대체된 ticket은 [`JobError::Superseded`]).
    Coalesced,
    /// background lane의 가장 오래된 job을 밀어내고 접수했습니다(밀려난 ticket은
    /// [`JobError::Evicted`]).
    EnqueuedEvicting,
}

/// 접수된 job의 ticket과 판정.
#[derive(Debug)]
pub struct Submitted<T> {
    /// 결과 handle.
    pub ticket: JobTicket<T>,
    /// 접수 판정.
    pub outcome: SubmitOutcome,
}

/// 다른 thread가 document thread에 job을 보내는 handle. 복제해 여러 thread에서 쓸 수 있습니다.
///
/// 문서 참조를 돌려주는 API가 없습니다. 모든 변경은 접수한 job 안에서 document thread가
/// 수행합니다. 접수 API는 용량·job 실행을 기다리지 않습니다(UI thread에서 안전).
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
    /// [`CommandPriority::Command`] 우선순위로 job을 접수합니다. 기다리지 않고 바로 반환합니다.
    ///
    /// command lane이 차 있으면 [`SubmitError::QueueFull`]입니다.
    pub fn submit<F, T>(&self, job: F) -> Result<JobTicket<T>, SubmitError>
    where
        F: FnOnce(&mut DocumentContext) -> T + Send + 'static,
        T: Send + 'static,
    {
        self.try_submit(SubmitOptions::command(), job)
            .map(|submitted| submitted.ticket)
    }

    /// 우선순위·병합 key를 지정해 job을 접수합니다. 기다리지 않고 overflow 정책대로 바로
    /// 판정합니다([`crate::command_queue`]).
    ///
    /// 대체·밀려난 이전 job의 ticket에는 lock 밖에서 이유를 알립니다.
    pub fn try_submit<F, T>(
        &self,
        options: SubmitOptions,
        job: F,
    ) -> Result<Submitted<T>, SubmitError>
    where
        F: FnOnce(&mut DocumentContext) -> T + Send + 'static,
        T: Send + 'static,
    {
        let (reply, receiver) = mpsc::channel();
        let job: Box<dyn ErasedJob> = Box::new(TypedJob { job, reply });
        let outcome = match self.inbox.try_push(options, job) {
            Ok(Pushed::Enqueued) => SubmitOutcome::Enqueued,
            Ok(Pushed::Coalesced { superseded }) => {
                superseded.discard(JobError::Superseded);
                SubmitOutcome::Coalesced
            }
            Ok(Pushed::EnqueuedEvicting { evicted }) => {
                evicted.discard(JobError::Evicted);
                SubmitOutcome::EnqueuedEvicting
            }
            Err(rejected) => {
                drop(rejected.item);
                return Err(match rejected.reason {
                    RejectReason::Closed => SubmitError::Stopped,
                    RejectReason::QueueFull { priority } => SubmitError::QueueFull { priority },
                });
            }
        };
        Ok(Submitted {
            ticket: JobTicket::new(receiver),
            outcome,
        })
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
    /// document thread가 종료 시간 안에 끝나지 않아 기다리지 않고 분리했습니다
    /// ([`DocumentThread::shutdown_timeout`]). 아직 시작하지 않은 job은 버렸습니다.
    ShutdownTimedOut {
        /// 기다린 최대 시간.
        timeout: Duration,
    },
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
            DocumentThreadError::ShutdownTimedOut { timeout } => write!(
                f,
                "document thread가 {timeout:?} 안에 끝나지 않아 기다리지 않고 분리했습니다"
            ),
        }
    }
}

impl std::error::Error for DocumentThreadError {}

/// document thread가 끝났는지(정상 종료·panic 모두) 알리는 신호.
#[derive(Debug, Default)]
struct ExitSignal {
    exited: Mutex<bool>,
    changed: Condvar,
}

impl ExitSignal {
    fn mark(&self) {
        *self.exited.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_all();
    }

    /// 최대 `timeout` 동안 thread 종료를 기다립니다. 끝났으면 `true`입니다.
    fn wait(&self, timeout: Duration) -> bool {
        let exited = self.exited.lock().unwrap_or_else(PoisonError::into_inner);
        let (exited, _) = self
            .changed
            .wait_timeout_while(exited, timeout, |exited| !*exited)
            .unwrap_or_else(PoisonError::into_inner);
        *exited
    }
}

/// thread가 어떤 경로로 끝나든(panic 포함) drop에서 [`ExitSignal`]을 세웁니다.
struct MarkExitOnDrop(Arc<ExitSignal>);

impl Drop for MarkExitOnDrop {
    fn drop(&mut self) {
        self.0.mark();
    }
}

/// 유일한 문서 작성자 thread와 그 event loop.
///
/// drop하면 [`DocumentThread::shutdown`]과 같이 접수를 닫고 남은 job을 처리한 뒤 join합니다.
pub struct DocumentThread {
    inbox: Arc<Inbox>,
    thread_id: ThreadId,
    join: Option<JoinHandle<Result<DocumentLoopStats, DocumentThreadError>>>,
    exit: Arc<ExitSignal>,
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
    /// 기본 lane 용량([`QueueCapacity::default`])으로 document thread를 시작합니다.
    pub fn spawn() -> Result<DocumentThread, DocumentThreadError> {
        DocumentThread::spawn_with(QueueCapacity::default())
    }

    /// lane 용량을 지정해 document thread를 시작합니다.
    pub fn spawn_with(capacity: QueueCapacity) -> Result<DocumentThread, DocumentThreadError> {
        let inbox = Arc::new(Inbox::new(capacity));
        let loop_inbox = Arc::clone(&inbox);
        let exit = Arc::new(ExitSignal::default());
        let loop_exit = MarkExitOnDrop(Arc::clone(&exit));
        let join = thread::Builder::new()
            .name(DOCUMENT_THREAD_NAME.to_string())
            .spawn(move || {
                let _exit = loop_exit;
                run_loop(loop_inbox)
            })
            .map_err(|error| DocumentThreadError::Spawn { kind: error.kind() })?;
        let thread_id = join.thread().id();
        Ok(DocumentThread {
            inbox,
            thread_id,
            join: Some(join),
            exit,
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
        self.stop_within(None)
    }

    /// [`DocumentThread::shutdown`]과 같이 종료하되 thread가 `timeout` 안에 끝나지 않으면 기다리지
    /// 않습니다.
    ///
    /// 시간을 넘기면 아직 시작하지 않은 job을 실행하지 않고 버리고(ticket은 [`JobError::Lost`]),
    /// thread를 분리한 뒤 [`DocumentThreadError::ShutdownTimedOut`]을 반환합니다. 실행 중인 job은
    /// 끊지 않으며, 그 job이 끝나면 thread가 스스로 끝납니다.
    pub fn shutdown_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<DocumentLoopStats, DocumentThreadError> {
        self.stop_within(Some(timeout))
    }

    /// 접수를 닫고 thread를 join합니다. `timeout`이 있으면 그 시간까지만 기다립니다.
    pub(crate) fn stop_within(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<DocumentLoopStats, DocumentThreadError> {
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
        if let Some(timeout) = timeout {
            if !self.exit.wait(timeout) {
                // 시작하지 않은 job은 lock 밖에서 버립니다. 실행 중인 job이 끝나면 수신함이 닫혀
                // 비었으므로 loop가 끝납니다.
                for job in self.inbox.close_and_drain() {
                    job.discard(JobError::Lost);
                }
                drop(join);
                return Err(DocumentThreadError::ShutdownTimedOut { timeout });
            }
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
            let _ = self.stop_within(None);
        }
    }
}

/// document thread event loop: 가장 높은 우선순위 lane의 가장 오래된 job부터 하나씩 실행합니다.
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

    /// command lane에 job을 접수합니다. lane이 차 있으면(`QueueFull`) 자리가 날 때까지 다시
    /// 시도합니다. 테스트가 lane 용량과 무관하게 단일 작성자 동작만 검사하도록 합니다.
    fn submit_retrying<F, T>(sender: &DocumentSender, make_job: impl Fn() -> F) -> JobTicket<T>
    where
        F: FnOnce(&mut DocumentContext) -> T + Send + 'static,
        T: Send + 'static,
    {
        loop {
            match sender.submit(make_job()) {
                Ok(ticket) => return ticket,
                Err(SubmitError::QueueFull { .. }) => thread::yield_now(),
                Err(error) => panic!("job 접수 실패: {error}"),
            }
        }
    }

    /// `threads`개 thread가 각각 `jobs_per_thread`개 변경 job을 동시에 보내고, 변경이 document
    /// thread 하나에서 순차 적용됐는지 검사합니다.
    fn assert_single_writer(capacity: QueueCapacity, threads: u64, jobs_per_thread: u64) {
        let doc = DocumentThread::spawn_with(capacity).unwrap();
        let sender = doc.sender();
        let handle = sender
            .submit(|ctx| ctx.open_document(stable(1)).handle())
            .unwrap()
            .wait()
            .unwrap();
        let senders: Vec<_> = (0..threads)
            .map(|t| {
                let sender = sender.clone();
                thread::spawn(move || {
                    let tickets: Vec<_> = (0..jobs_per_thread)
                        .map(|i| {
                            submit_retrying(&sender, || {
                                move |ctx: &mut DocumentContext| {
                                    let mut doc = ctx.document_mut(handle).expect("열린 문서");
                                    doc.insert_node(fields(t * 1_000_000 + i + 10));
                                    (t, doc.revision())
                                }
                            })
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
        for t in 0..threads {
            let revisions: Vec<u64> = applied
                .iter()
                .filter(|(owner, _)| *owner == t)
                .map(|(_, revision)| revision.0)
                .collect();
            assert_eq!(revisions.len() as u64, jobs_per_thread);
            assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
        }
        // 단일 작성자: 모든 변경이 서로 다른 revision 1..=total을 하나씩 받습니다.
        let total = threads * jobs_per_thread;
        let mut revisions: Vec<u64> = applied.iter().map(|(_, revision)| revision.0).collect();
        revisions.sort_unstable();
        assert_eq!(revisions, (1..=total).collect::<Vec<u64>>());
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
        assert_eq!(count, usize::try_from(total).ok());
        assert_eq!(revision, Some(DocumentRevision(total)));
        doc.shutdown().unwrap();
    }

    #[test]
    fn concurrent_senders_mutate_only_through_the_single_writer() {
        // 4개 thread가 동시에 job을 보내도 변경은 document thread 하나가 순차 적용합니다.
        assert_single_writer(QueueCapacity::default(), 4, 250);
    }

    #[test]
    fn single_writer_holds_when_the_command_lane_is_smaller_than_the_load() {
        // 회귀(R5): 접수 수가 command lane 용량을 넘어도 `QueueFull` panic 없이 단일 작성자
        // 동작만 검사합니다. lane 8개에 4×300 = 1,200건(기본 용량 1024도 초과)을 보냅니다.
        let capacity = QueueCapacity::new(4, 8, 4).unwrap();
        assert_single_writer(capacity, 4, 300);
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
    fn shutdown_timeout_detaches_a_stalled_thread_and_discards_pending_jobs() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let (entered, started) = mpsc::channel::<()>();
        let (release, wait) = mpsc::channel::<()>();
        let stalled = sender
            .submit(move |_ctx| {
                let _ = entered.send(());
                let _ = wait.recv();
                1u32
            })
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let pending = sender.submit(|_ctx| 2u32).unwrap();

        let timeout = Duration::from_millis(100);
        let begun = Instant::now();
        assert_eq!(
            doc.shutdown_timeout(timeout),
            Err(DocumentThreadError::ShutdownTimedOut { timeout })
        );
        assert!(begun.elapsed() < Duration::from_secs(5));
        assert!(sender.is_stopped());
        // 시작하지 않은 job은 실행하지 않고 버립니다.
        assert_eq!(pending.wait(), Err(TicketError::Job(JobError::Lost)));
        // 실행 중이던 job은 끊지 않습니다. 풀어 주면 끝나고 결과를 돌려줍니다.
        release.send(()).unwrap();
        assert_eq!(stalled.wait(), Ok(1));
    }

    #[test]
    fn shutdown_timeout_joins_a_thread_that_finishes_in_time() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let tickets: Vec<_> = (0..5u32)
            .map(|i| sender.submit(move |_ctx| i).unwrap())
            .collect();
        let stats = doc.shutdown_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(stats.jobs_run, 5, "시간 안이면 접수된 job을 모두 실행");
        let values: Vec<u32> = tickets.into_iter().map(|t| t.wait().unwrap()).collect();
        assert_eq!(values, (0..5).collect::<Vec<u32>>());
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
        assert!(DocumentThreadError::ShutdownTimedOut {
            timeout: Duration::from_secs(2)
        }
        .to_string()
        .contains("분리"));
    }
}

/// XII.1.2: bounded 우선순위 수신함과 UI 무대기 접수.
#[cfg(test)]
mod queue_tests {
    use super::*;
    use crate::command_queue::InputEventClass;
    use std::sync::Mutex;
    use std::time::Instant;

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

    /// document thread를 job 실행 중 상태로 막아 두고, 수신함이 빌 때까지 기다립니다.
    fn block_document_thread(sender: &DocumentSender) -> Sender<()> {
        let (release, wait) = mpsc::channel::<()>();
        sender
            .submit(move |_ctx| {
                let _ = wait.recv();
            })
            .unwrap();
        assert!(
            wait_until(|| sender.inbox.is_empty()),
            "gate job이 꺼내져 실행 중이어야 함"
        );
        release
    }

    /// `ThreadRole::Ui`로 bind한 thread에서 body를 실행합니다.
    fn on_ui_thread<R: Send + 'static>(
        body: impl FnOnce() -> R + Send + 'static,
    ) -> thread::JoinHandle<R> {
        thread::spawn(move || {
            let _ui = bind_thread_role(ThreadRole::Ui).unwrap();
            body()
        })
    }

    type Log = Arc<Mutex<Vec<String>>>;

    fn logging_job(log: &Log, label: &str) -> impl FnOnce(&mut DocumentContext) + Send + 'static {
        let log = Arc::clone(log);
        let label = label.to_string();
        move |_ctx| log.lock().unwrap().push(label)
    }

    #[test]
    fn ui_submissions_never_wait_for_a_busy_document_thread() {
        // 검증 기준(XII.1.2) "blocking UI call 없음": document thread가 job 실행 중으로 멈춰 있어도
        // UI thread의 접수는 overflow 정책대로 바로 판정되고, UI thread의 blocking 대기는 거부됩니다.
        let capacity = QueueCapacity::new(4, 4, 4).unwrap();
        let doc = DocumentThread::spawn_with(capacity).unwrap();
        let sender = doc.sender();
        let release = block_document_thread(&sender);
        let log: Log = Arc::new(Mutex::new(Vec::new()));

        let (done_tx, done_rx) = mpsc::channel::<()>();
        let ui_sender = sender.clone();
        let ui_log = Arc::clone(&log);
        let ui = on_ui_thread(move || {
            let mut outcomes = Vec::new();
            let mut tickets = Vec::new();
            for (prefix, options) in [
                ("i", SubmitOptions::input()),
                ("c", SubmitOptions::command()),
                ("b", SubmitOptions::background()),
            ] {
                for n in 0..6 {
                    let label = format!("{prefix}{n}");
                    match ui_sender.try_submit(options, logging_job(&ui_log, &label)) {
                        Ok(submitted) => {
                            outcomes.push((label.clone(), Ok(submitted.outcome)));
                            tickets.push((label, submitted.ticket));
                        }
                        Err(error) => outcomes.push((label, Err(error))),
                    }
                }
            }
            // UI thread: 대기 없는 확인만 허용, blocking 대기는 거부.
            let (_, first) = &mut tickets[0];
            let polled = first.try_take();
            let timed = first.wait_timeout(Duration::from_secs(5));
            let (_, last) = tickets.pop().unwrap();
            let waited = last.wait();
            done_tx.send(()).unwrap();
            (outcomes, tickets, polled, timed, waited)
        });
        // gate를 열기 전에 UI thread가 끝나야 합니다(접수 중 대기 없음).
        assert!(
            done_rx.recv_timeout(Duration::from_secs(10)).is_ok(),
            "UI 접수가 바쁜 document thread를 기다렸습니다"
        );
        let (outcomes, tickets, polled, timed, waited) = ui.join().unwrap();

        let refused = TicketError::BlockingNotAllowed {
            role: ThreadRole::Ui,
        };
        assert_eq!(polled, Ok(None));
        assert_eq!(timed, Err(refused));
        assert_eq!(waited, Err(refused));

        let expected: Vec<(String, Result<SubmitOutcome, SubmitError>)> = [
            ("i", CommandPriority::Input),
            ("c", CommandPriority::Command),
            ("b", CommandPriority::Background),
        ]
        .into_iter()
        .flat_map(|(prefix, priority)| {
            (0..6).map(move |n| {
                let outcome = match (priority, n) {
                    (_, 0..=3) => Ok(SubmitOutcome::Enqueued),
                    (CommandPriority::Background, _) => Ok(SubmitOutcome::EnqueuedEvicting),
                    (priority, _) => Err(SubmitError::QueueFull { priority }),
                };
                (format!("{prefix}{n}"), outcome)
            })
        })
        .collect();
        assert_eq!(outcomes, expected);

        release.send(()).unwrap();
        let stats = doc.shutdown().unwrap();
        // gate 1 + input 4 + command 4 + background 4(b5는 UI가 ticket을 버렸어도 실행).
        assert_eq!(stats.jobs_run, 13);
        assert_eq!(
            *log.lock().unwrap(),
            ["i0", "i1", "i2", "i3", "c0", "c1", "c2", "c3", "b2", "b3", "b4", "b5"]
        );
        // 남은 ticket(b5는 UI thread의 거부된 wait가 소비)은 역할 없는 test thread에서 기다립니다.
        // i0은 UI thread의 try_take·wait_timeout이 결과를 꺼내지 않았으므로 여전히 결과를 받습니다.
        for (label, ticket) in tickets {
            let result = ticket.wait();
            match label.as_str() {
                "b0" | "b1" => assert_eq!(result, Err(TicketError::Job(JobError::Evicted))),
                _ => assert_eq!(result, Ok(()), "{label}"),
            }
        }
    }

    #[test]
    fn pointer_moves_coalesce_to_the_latest_while_busy() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let release = block_document_thread(&sender);
        let options = InputEventClass::PointerMove { device: 1 }.submit_options();
        let mut submitted: Vec<Submitted<u32>> = (0..100u32)
            .map(|x| sender.try_submit(options, move |_ctx| x).unwrap())
            .collect();
        assert_eq!(submitted[0].outcome, SubmitOutcome::Enqueued);
        assert!(submitted[1..]
            .iter()
            .all(|s| s.outcome == SubmitOutcome::Coalesced));
        assert_eq!(sender.inbox.lane_len(CommandPriority::Input), 1);
        release.send(()).unwrap();
        let last = submitted.pop().unwrap();
        assert_eq!(last.ticket.wait(), Ok(99));
        for earlier in submitted {
            assert_eq!(
                earlier.ticket.wait(),
                Err(TicketError::Job(JobError::Superseded))
            );
        }
        assert_eq!(doc.shutdown().unwrap().jobs_run, 2);
    }

    #[test]
    fn discrete_input_is_never_merged_or_reordered() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let release = block_document_thread(&sender);
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let steps = [
            ("m1", InputEventClass::PointerMove { device: 1 }),
            ("m2", InputEventClass::PointerMove { device: 1 }),
            ("down", InputEventClass::Discrete),
            ("m3", InputEventClass::PointerMove { device: 1 }),
            ("w1", InputEventClass::Wheel),
            ("w2", InputEventClass::Wheel),
            ("r1", InputEventClass::Resize),
            ("r2", InputEventClass::Resize),
            ("up", InputEventClass::Discrete),
        ];
        let outcomes: Vec<SubmitOutcome> = steps
            .iter()
            .map(|(label, class)| {
                sender
                    .try_submit(class.submit_options(), logging_job(&log, label))
                    .unwrap()
                    .outcome
            })
            .collect();
        use SubmitOutcome::{Coalesced, Enqueued};
        assert_eq!(
            outcomes,
            [
                Enqueued, Coalesced, Enqueued, Enqueued, Enqueued, Enqueued, Enqueued, Coalesced,
                Enqueued
            ]
        );
        release.send(()).unwrap();
        doc.shutdown().unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            ["m2", "down", "m3", "w1", "w2", "r2", "up"]
        );
    }

    #[test]
    fn submit_uses_the_command_lane_after_input() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        let release = block_document_thread(&sender);
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        sender
            .try_submit(SubmitOptions::background(), logging_job(&log, "bg"))
            .unwrap();
        sender.submit(logging_job(&log, "cmd")).unwrap();
        sender
            .try_submit(SubmitOptions::input(), logging_job(&log, "key"))
            .unwrap();
        assert_eq!(sender.inbox.lane_len(CommandPriority::Command), 1);
        release.send(()).unwrap();
        doc.shutdown().unwrap();
        assert_eq!(*log.lock().unwrap(), ["key", "cmd", "bg"]);
    }

    #[test]
    fn stopped_thread_rejects_ui_submissions_immediately() {
        let doc = DocumentThread::spawn().unwrap();
        let sender = doc.sender();
        doc.shutdown().unwrap();
        let result = on_ui_thread(move || {
            sender
                .try_submit(SubmitOptions::input(), |_ctx| ())
                .map(|s| s.outcome)
        })
        .join()
        .unwrap();
        assert_eq!(result, Err(SubmitError::Stopped));
    }

    #[test]
    fn queue_errors_display_in_korean() {
        assert!(SubmitError::QueueFull {
            priority: CommandPriority::Input
        }
        .to_string()
        .contains("input lane"));
        assert!(JobError::Superseded.to_string().contains("대체"));
        assert!(JobError::Evicted.to_string().contains("밀려"));
        assert!(TicketError::BlockingNotAllowed {
            role: ThreadRole::Ui
        }
        .to_string()
        .contains("Ui"));
    }
}
