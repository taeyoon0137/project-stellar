# ADR-0028: 창 session과 셸 명령 C ABI

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §22.2는 같은 프로젝트를 여러 창에서 열 수 있고 각 창이 독립된 인메모리 문서 상태와
Undo/Redo 스택을 가지도록 정합니다. §23.2는 창별로 독립된 history를, §8.2는 handle·POD·버전형 메시지만
쓰는 셸 경계를 요구합니다. 구현 계획 XII.2.1은 코어에 `ProjectWindowSession`(ProjectIdentity,
WindowSessionId, 독립 document state, view state, history, loaded generation)을 두고, XII.2.2는 macOS
`NSWindowController`가 그 handle 하나를 소유하도록 연결합니다.

기존 C ABI(ADR-0009, ADR-0014)는 프로젝트 session(`stellar_project_*`)과 빈 문서 session
(`stellar_document_*`)만 노출합니다. 창 하나가 소유할 편집 session의 handle과 그 상태를 읽는 경로가 없습니다.
구현 계획 1.2의 3번 규칙과 I.3.5 정책에 따라 `crates/ffi/` 변경을 이 ADR에 행별로 기록합니다. 기존 함수·
상태 코드·event의 의미는 바꾸지 않습니다.

## Decision

### XII.2.2 — 창 session handle

1. **코어 표**: `stellar-runtime`의 document thread session 표에 창 session(`ProjectWindowSession`)을
   둡니다. 창 session은 열린 프로젝트 session 아래에서만 열리고, 프로젝트 session을 닫으면 그 아래 창
   session도 폐기합니다. handle은 runtime 정수(`WindowSessionHandle`, 1부터 증가, 재사용 없음)이고, 영속
   식별자 `WindowSessionId`(UUIDv7, 복구 snapshot 키 V.9.1)와 별개입니다.
2. **함수**(모두 기다리지 않고 request id를 돌려줌, 완료는 pull event):
   - `stellar_window_session_open(handle, project, out_request)` → `STELLAR_EVENT_WINDOW_SESSION_OPENED`
     (`0x0106`: request, project, session, window_id\[16\], document_id\[16\], generation) 또는
     `REQUEST_FAILED`.
   - `stellar_window_session_close(handle, session, out_request)` → `STELLAR_EVENT_WINDOW_SESSION_CLOSED`
     (`0x0107`: request, session). 닫으면 저장하지 않은 편집·history·선택을 폐기합니다.
   - `stellar_window_session_info(handle, session, out_info)`: 최신 상태 snapshot에서 고정 88 byte
     `StellarWindowSessionInfo`(version 1: has_document, session, project, revision, generation,
     undo_depth, redo_depth, selection_count, window_id, document_id)를 씁니다. document thread를 기다리지
     않습니다. 없는 session은 `STELLAR_INVALID_ARGUMENT`.
3. **실패 코드**(끝에 덧붙임): `STELLAR_FAILURE_UNKNOWN_WINDOW_SESSION`(4, 문서 session 실패 코드 옆
   빈 번호), `STELLAR_FAILURE_WINDOW_SESSION_LOAD_FAILED`(18, current generation·작업 문서 로드 실패).
4. **`StellarRuntimeState` version 2**: 끝에 `window_session_count`(u64)를 더해 48 byte가 됩니다. 셸은
   `version == STELLAR_RUNTIME_STATE_VERSION`을 확인합니다.
5. **작업 문서**: 창 session은 current generation의 첫 root 문서 하나를 작업 문서로 로드합니다. 저장한 적
   없는 프로젝트(current pointer 없음)는 작업 문서 없이(document_id 0, generation 0) 엽니다. 문서 전환은
   이 결정의 범위 밖입니다.
6. **셸 소유**: macOS `ShellWindowController.windowSession`이 창 session handle 하나를 소유합니다. 셸은
   프로젝트 session을 연 뒤 창 session까지 열린 다음에 프로젝트 창을 ready로 두고, 창 session이 실패하면
   프로젝트 session을 닫고 열기 실패로 안내합니다. 창을 닫으면 창 session → 프로젝트 session 순서로 한 번씩
   닫습니다.

## Alternatives

- **프로젝트 session handle에 창 상태를 합침**: 같은 프로젝트를 연 두 창(XII.2.4)이 독립 문서·history를
  가져야 하므로 프로젝트 session 하나에 묶을 수 없습니다. 채택하지 않습니다.
- **창 session 열기를 프로젝트 열기 요청에 포함**: 복원·Locate Folder·다시 시도처럼 같은 창에서 프로젝트를
  다시 여는 경로와 같은 프로젝트의 둘째 창(XII.2.4)이 따로 창 session을 열어야 합니다. 요청을 나눕니다.
- **`StellarRuntimeState`를 그대로 두고 개수 조회 함수를 추가**: 상태 snapshot 요약을 한 번에 읽는 기존
  규칙(ADR-0014)과 맞지 않습니다. version을 올려 필드를 덧붙입니다.
- **영속 `WindowSessionId`를 FFI handle로 사용**: 16 byte id를 매 호출에 넘기게 되고, 닫은 뒤 같은 id를
  다시 쓸 수 있는지 규칙이 필요해집니다. runtime 정수 handle을 쓰고 id는 정보로만 노출합니다.

## Consequences

- 긍정: 창 controller 하나가 코어 창 session 하나를 소유하는 경로가 생기고, 창마다 문서·history·선택이
  분리됩니다(XII.2.1 검증).
- 긍정: 이후 창별 명령(Undo/Redo/Save Now, XII.6.4)과 view state(XII.2.5)를 같은 handle에 붙일 수 있습니다.
- 부정: 창 session은 열 때 작업 문서 전체를 document thread에서 로드합니다. 큰 문서의 lazy 로드와 비활성
  문서 unload(XVI.5.5)는 창 session에 아직 연결하지 않았습니다.
- 한계: 같은 프로젝트를 두 창에서 여는 셸 정책(XII.2.4)과 저장 충돌 정책(XII.7)은 바뀌지 않았습니다.

## Compatibility

파일 포맷·저장 구조에 영향이 없습니다. C ABI는 함수·event kind·실패 코드를 덧붙이고
`StellarRuntimeState`를 version 2(40 → 48 byte)로 올립니다. 기존 함수 시그니처와 event payload는 그대로이며,
헤더(`cargo xtask generate-ffi-header`)와 Rust 테스트가 크기·offset·상수를 고정합니다.

## Performance

성능 예산(`config/performance-budgets.yaml`)은 바꾸지 않습니다. 정보 조회는 기존 상태 snapshot 읽기와 같은
O(1) 임계 구역입니다. 창 session 열기의 generation·문서 로드는 document thread의 Command 우선순위 job이며
UI thread는 기다리지 않습니다.

## Security

새 외부 의존성과 `unsafe` 패턴이 없습니다(기존 포인터 규칙 그대로). 신뢰되지 않은 프로젝트도 창 session은
저장된 design chunk만 읽고 Git·필터를 실행하지 않습니다.

## Migration

기존 데이터 migration은 없습니다. 셸은 새 헤더로 다시 빌드합니다.
