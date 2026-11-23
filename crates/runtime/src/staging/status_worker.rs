//! Git 상태를 worker pool에서 계산합니다(UX.5.1, ADR-0080 결정 5).
//!
//! 호출 thread(앱 main·document thread)는 Git 프로세스를 만들지 않습니다. job은 worker thread에서
//! [`GitSession::status`]를 실행하고, 결과에 요청 순번(`sequence`)과 계산 revision을 붙여 channel로 보냅니다.
//! 받는 쪽은 마지막으로 반영한 순번보다 작은 결과를 버립니다. 결과는 디스크에 저장된 파일 기준입니다.
//!
//! worker class는 아직 Git 전용 class가 없으므로 [`WorkerClass::CacheGeneration`](파생 상태 계산,
//! Background)을 씁니다. 상태 조회가 저장 직렬화를 앞지르지 않습니다.

use std::sync::mpsc::Sender;

use stellar_ops::git::{GitError, GitSession, GitStatusReport, StatusRequest};

use crate::worker_pool::{WorkerClass, WorkerJobId, WorkerJobSpec, WorkerPool, WorkerRejected};

/// 상태 조회 job 입력.
#[derive(Debug, Clone)]
pub struct GitStatusJob {
    /// 요청 순번(호출자가 단조 증가로 매김).
    pub sequence: u64,
    /// worktree root에 고정한 세션.
    pub session: GitSession,
    /// page 요청.
    pub request: StatusRequest,
}

/// 상태 조회 결과.
#[derive(Debug)]
pub struct GitStatusResult {
    /// 요청 순번.
    pub sequence: u64,
    /// 계산한 thread 이름(검증·진단용).
    pub thread: Option<String>,
    /// 상태. 성공하면 `revision`을 담습니다.
    pub status: Result<GitStatusReport, GitError>,
}

/// 접수 거부 때 돌려받는 job과 응답 channel.
pub type RejectedGitStatus = WorkerRejected<Box<(GitStatusJob, Sender<GitStatusResult>)>>;

/// 상태 조회를 worker에 접수합니다. 큐가 차면 job과 응답 channel을 돌려받아 나중에 다시 제출합니다.
pub fn submit_git_status(
    pool: &WorkerPool,
    job: GitStatusJob,
    reply: Sender<GitStatusResult>,
) -> Result<WorkerJobId, RejectedGitStatus> {
    pool.try_submit_with(
        WorkerJobSpec::new(WorkerClass::CacheGeneration),
        Box::new((job, reply)),
        |payload, _context| {
            let (job, reply) = *payload;
            let status = job.session.status(&job.request);
            let _ = reply.send(GitStatusResult {
                sequence: job.sequence,
                thread: std::thread::current().name().map(str::to_owned),
                status,
            });
        },
    )
}
