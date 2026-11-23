//! 내장 에이전트 구독 CLI 호스트 코어 (`stellar-agent-host`, XXIX.2, ADR-0005 §4·§6·§8, ADR-0092).
//!
//! 사용자가 직접 설치하고 로그인한 공급자 공식 CLI를 각 CLI 공식 문서의 비대화형 인터페이스로 실행하고,
//! 그 출력을 표시 전용으로 디코드합니다. 문서 변경은 CLI가 호출한 Stellar MCP tool이 앱 승인·lease·
//! DocumentThread를 거쳐서만 일으키며(ADR-0005 §4.3 불변식 1), 이 crate에는 문서·명령·history·저장·ops
//! 타입이 없고 그 crate에 도달하지도 않습니다(§8).
//!
//! Stellar는 모델 제공자 API client, API key, 자격 증명 저장, 모델 라우팅, 사용량 계량을 두지 않습니다
//! (§4.1). 이 crate는 CLI 자격 증명 저장 위치를 읽거나 열지 않습니다(`tests/inventory.rs`).
//!
//! | 모듈 | 행 | 역할 |
//! |---|---|---|
//! | [`json`] | XXIX.2.6·XXIX.2.11 | 엄격한 JSON reader·writer(mcp-core 사본) |
//! | [`profile`] | XXIX.2.6 | Stellar 소유 launch profile fixture와 검증 |
//! | [`target`] | XXIX.2.3 | 사용자가 확인한 실행 대상과 코드 서명 신원 확인 |
//! | [`env`] | XXIX.2.4 | 허용 목록 child 환경(ADR-0023 결정 9) |
//! | [`process`] | XXIX.2.4 | macOS 실행기(파이프, process group, timeout·취소, 흔적 정리) |
//! | [`status`] | XXIX.2.5 | 공식 방법만 쓰는 연결 상태 판정(fail closed) |
//! | [`preflight`] | XXIX.2.7 | 세션 실행 전 확인(fail closed) |
//! | [`session`] | XXIX.2.8·XXIX.2.13 | AgentSession 상태 기계와 XIX.6.4 종료 절차 |
//! | [`chat`] | XXIX.2.10 | 채팅 보기 모델 `AgentChatItem` closed enum |
//! | [`decoder`] | XXIX.2.11 | launch profile별 표시 전용 출력 디코더(fail closed) |
//! | [`raw`] | XXIX.2.12 | 세션별 메모리 상한 원문 버퍼 |
//! | [`output`] | XXIX.2.4·XXIX.2.13 | 수신 출력의 원문 보존·디코드·디코드 실패 연결 |
//! | `src/bin/test_cli.rs` | XXIX.2.2 | 구독 CLI 테스트 대역 실행 파일(테스트 전용, library 미사용) |

pub mod chat;
pub mod decoder;
pub mod env;
pub mod json;
pub mod output;
pub mod process;
pub mod profile;
pub mod raw;
pub mod target;
