//! 비동기 request/response·batch RPC session (XXI.4.3, ADR-0097 결정 7).
//!
//! [`HostSession::spawn`]은 session마다 worker thread 하나를 두고 [`Dispatcher`]를 그 thread에 넘깁니다.
//! [`HostSession::submit`]은 메시지 형식·크기·호출량·처리 중 수를 바로 검사하고 즉시 돌아옵니다. 통과한 메시지는
//! 제출 순서대로 worker가 실행하고, 응답(거부 응답 포함)은 모두 [`HostSession::recv_timeout`]으로 받습니다.
//!
//! 메시지 형식:
//!
//! - 요청 `{"stellar_rpc":1,"id":<정수|문자열>,"method":"ns.name","params":{…}}`
//! - batch `{"stellar_rpc":1,"id":…,"batch":[{"method":…,"params":…},…]}`(항목은 순서대로, 앞 항목 실패와 무관하게 실행)
//! - 응답 `{"stellar_rpc":1,"id":…,"result":…}` 또는 `{"stellar_rpc":1,"id":…,"error":{"code","message","data"?}}`,
//!   batch 응답 `{"stellar_rpc":1,"id":…,"batch":[{"result":…}|{"error":…},…]}`
//!
//! 호출량은 1초 창의 메시지 수로 셉니다(batch는 1개). 60Hz pointer·render loop처럼 매 frame 부르는 호출은 첫 1초 안에
//! `rate-limited`가 됩니다. 시간은 주입한 [`Clock`]으로 재므로 fixture가 결정적입니다.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::host::Dispatcher;
use crate::json::{parse, write, JsonLimits, JsonNumber, JsonValue as J};
use crate::permission::Permission;
use crate::rpc::{RpcError, RpcLimits};

/// envelope version.
pub const STELLAR_RPC_VERSION: i64 = 1;

/// 문자열 id 최대 길이.
pub const MAX_ID_CHARS: usize = 64;

/// 호출량 창 길이.
pub const RATE_WINDOW: Duration = Duration::from_secs(1);

/// 단조 시계.
pub trait Clock: Send + Sync {
    /// 기준 시점 이후 경과 시간.
    fn now(&self) -> Duration;
}

/// 실제 단조 시계.
#[derive(Debug)]
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// 제출 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submitted {
    /// worker queue에 넣었습니다.
    Queued,
    /// 바로 거부했습니다(거부 응답은 응답 channel에 있습니다).
    Rejected(&'static str),
}

enum Job {
    Call { id: J, method: String, params: J },
    Batch { id: J, items: Vec<J> },
    SetPermissions(BTreeSet<Permission>),
}

struct Admission {
    recent: VecDeque<Duration>,
}

/// 한 플러그인 session.
pub struct HostSession {
    jobs: Option<Sender<Job>>,
    responses: Receiver<String>,
    reply: Sender<String>,
    admission: Mutex<Admission>,
    in_flight: Arc<AtomicUsize>,
    clock: Arc<dyn Clock>,
    limits: RpcLimits,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for HostSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostSession")
            .field("in_flight", &self.in_flight.load(Ordering::SeqCst))
            .field("limits", &self.limits)
            .finish()
    }
}

/// 응답 envelope.
pub fn envelope(id: &J, body: (&str, J)) -> String {
    write(&J::object(vec![
        ("stellar_rpc", J::int(STELLAR_RPC_VERSION)),
        ("id", id.clone()),
        body,
    ]))
}

fn error_reply(id: &J, error: &RpcError) -> String {
    envelope(id, ("error", error.to_json()))
}

fn valid_id(id: &J) -> bool {
    match id {
        J::Number(n) => n.is_integer() && !n.as_str().starts_with('-'),
        J::String(s) => !s.is_empty() && s.chars().count() <= MAX_ID_CHARS,
        _ => false,
    }
}

/// batch 항목 하나를 실행합니다.
fn run_item(dispatcher: &Dispatcher, item: &J) -> J {
    let outcome = (|| {
        let pairs = item
            .as_object()
            .ok_or_else(|| RpcError::new("invalid-request", "batch 항목은 object여야 합니다"))?;
        if pairs.iter().any(|(k, _)| k != "method" && k != "params") {
            return Err(RpcError::new(
                "invalid-request",
                "batch 항목에는 method와 params만 둡니다",
            ));
        }
        let method = item
            .get("method")
            .and_then(J::as_str)
            .ok_or_else(|| RpcError::new("invalid-request", "method 문자열이 필요합니다"))?;
        let empty = J::Object(Vec::new());
        dispatcher.call(method, item.get("params").unwrap_or(&empty))
    })();
    match outcome {
        Ok(result) => J::object(vec![("result", result)]),
        Err(error) => J::object(vec![("error", error.to_json())]),
    }
}

impl HostSession {
    /// worker thread를 띄웁니다.
    pub fn spawn(dispatcher: Dispatcher, clock: Arc<dyn Clock>) -> HostSession {
        HostSession::spawn_with(dispatcher, clock, RpcLimits::default())
    }

    /// 상한을 지정해 띄웁니다(test용으로 더 작은 상한).
    pub fn spawn_with(
        mut dispatcher: Dispatcher,
        clock: Arc<dyn Clock>,
        limits: RpcLimits,
    ) -> HostSession {
        let (jobs, inbox) = channel::<Job>();
        let (reply, responses) = channel::<String>();
        let in_flight = Arc::new(AtomicUsize::new(0));
        let worker_reply = reply.clone();
        let worker_in_flight = Arc::clone(&in_flight);
        let chunk_bytes = limits.chunk_bytes;
        let worker = std::thread::Builder::new()
            .name("stellar-plugin-host".to_owned())
            .spawn(move || {
                for job in inbox {
                    let messages = match job {
                        Job::SetPermissions(granted) => {
                            dispatcher.set_permissions(&granted);
                            continue;
                        }
                        Job::Call { id, method, params } => {
                            let reply = match dispatcher.call(&method, &params) {
                                Ok(result) => envelope(&id, ("result", result)),
                                Err(error) => error_reply(&id, &error),
                            };
                            crate::rpc::chunk_reply(&id, reply, chunk_bytes)
                        }
                        Job::Batch { id, items } => {
                            let results = items.iter().map(|i| run_item(&dispatcher, i)).collect();
                            let reply = envelope(&id, ("batch", J::Array(results)));
                            crate::rpc::chunk_reply(&id, reply, chunk_bytes)
                        }
                    };
                    worker_in_flight.fetch_sub(1, Ordering::SeqCst);
                    for message in messages {
                        if worker_reply.send(message).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("worker thread");
        HostSession {
            jobs: Some(jobs),
            responses,
            reply,
            admission: Mutex::new(Admission {
                recent: VecDeque::new(),
            }),
            in_flight,
            clock,
            limits,
            worker: Some(worker),
        }
    }

    /// 상한.
    pub fn limits(&self) -> RpcLimits {
        self.limits
    }

    /// 처리 중 메시지 수.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    fn reject(&self, id: &J, error: RpcError) -> Submitted {
        let code = error.code;
        let _ = self.reply.send(error_reply(id, &error));
        Submitted::Rejected(code)
    }

    /// 메시지 하나를 제출합니다. 바로 돌아옵니다.
    pub fn submit(&self, message: &str) -> Submitted {
        if message.len() > self.limits.max_message_bytes {
            return self.reject(
                &J::Null,
                RpcError::new("too-large", "메시지 크기 상한을 넘습니다"),
            );
        }
        let Ok(value) = parse(
            message,
            JsonLimits {
                max_bytes: self.limits.max_message_bytes,
                ..JsonLimits::default()
            },
        ) else {
            return self.reject(
                &J::Null,
                RpcError::new("invalid-request", "JSON이 아닙니다"),
            );
        };
        let id = value.get("id").cloned().unwrap_or(J::Null);
        if !valid_id(&id) {
            return self.reject(
                &J::Null,
                RpcError::new(
                    "invalid-request",
                    "id는 음이 아닌 정수나 64자 이하 문자열이어야 합니다",
                ),
            );
        }
        let Some(pairs) = value.as_object() else {
            return self.reject(
                &id,
                RpcError::new("invalid-request", "메시지는 object여야 합니다"),
            );
        };
        if value.get("stellar_rpc").and_then(J::as_i64) != Some(STELLAR_RPC_VERSION) {
            return self.reject(
                &id,
                RpcError::new("unsupported-rpc-version", "stellar_rpc 1만 받습니다"),
            );
        }
        let has = |k: &str| value.get(k).is_some();
        let allowed: &[&str] = if has("batch") {
            &["stellar_rpc", "id", "batch"]
        } else {
            &["stellar_rpc", "id", "method", "params"]
        };
        if pairs.iter().any(|(k, _)| !allowed.contains(&k.as_str())) {
            return self.reject(
                &id,
                RpcError::new("invalid-request", "envelope에 모르는 key가 있습니다"),
            );
        }
        let job = if let Some(batch) = value.get("batch") {
            let Some(items) = batch.as_array() else {
                return self.reject(
                    &id,
                    RpcError::new("invalid-request", "batch는 배열이어야 합니다"),
                );
            };
            if items.is_empty() {
                return self.reject(&id, RpcError::new("invalid-request", "batch가 비었습니다"));
            }
            if items.len() > self.limits.max_batch_items {
                return self.reject(
                    &id,
                    RpcError::new("batch-too-large", "batch 항목 수 상한을 넘습니다").with_data(
                        J::object(vec![(
                            "max_batch_items",
                            J::Number(JsonNumber::from_u64(self.limits.max_batch_items as u64)),
                        )]),
                    ),
                );
            }
            Job::Batch {
                id: id.clone(),
                items: items.to_vec(),
            }
        } else {
            let Some(method) = value.get("method").and_then(J::as_str) else {
                return self.reject(
                    &id,
                    RpcError::new("invalid-request", "method 문자열이 필요합니다"),
                );
            };
            Job::Call {
                id: id.clone(),
                method: method.to_owned(),
                params: value
                    .get("params")
                    .cloned()
                    .unwrap_or(J::Object(Vec::new())),
            }
        };

        {
            let now = self.clock.now();
            let mut admission = self.admission.lock().expect("admission lock");
            while admission
                .recent
                .front()
                .is_some_and(|t| now.saturating_sub(*t) >= RATE_WINDOW)
            {
                admission.recent.pop_front();
            }
            if admission.recent.len() >= self.limits.max_messages_per_second {
                let oldest = *admission.recent.front().expect("비어 있지 않음");
                let retry = (oldest + RATE_WINDOW).saturating_sub(now);
                drop(admission);
                return self.reject(
                    &id,
                    RpcError::new(
                        "rate-limited",
                        "호출량 상한을 넘었습니다. 고빈도 loop에서 Host RPC를 부르지 말고 batch로 묶으세요",
                    )
                    .with_data(J::object(vec![(
                        "retry_after_ms",
                        J::Number(JsonNumber::from_u64(retry.as_millis().max(1) as u64)),
                    )])),
                );
            }
            if self.in_flight.load(Ordering::SeqCst) >= self.limits.max_in_flight {
                drop(admission);
                return self.reject(
                    &id,
                    RpcError::new("too-many-in-flight", "처리 중 요청 수 상한을 넘었습니다"),
                );
            }
            admission.recent.push_back(now);
            self.in_flight.fetch_add(1, Ordering::SeqCst);
        }
        match self.jobs.as_ref().map(|jobs| jobs.send(job)) {
            Some(Ok(())) => Submitted::Queued,
            _ => {
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                self.reject(&id, RpcError::new("closed", "session이 닫혔습니다"))
            }
        }
    }

    /// 유효 권한을 바꿉니다(승인·철회). 이미 queue에 있는 요청 뒤에 반영됩니다.
    pub fn set_permissions(&self, granted: BTreeSet<Permission>) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::SetPermissions(granted));
        }
    }

    /// 응답 하나를 기다립니다.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<String> {
        self.responses.recv_timeout(timeout).ok()
    }

    /// 기다리지 않고 응답 하나를 가져옵니다.
    pub fn try_recv(&self) -> Option<String> {
        self.responses.try_recv().ok()
    }

    /// 새 요청을 받지 않고 남은 요청을 끝낸 뒤 worker를 멈춥니다.
    pub fn close(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        self.close();
    }
}
