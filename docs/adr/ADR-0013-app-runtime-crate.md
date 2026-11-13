# ADR-0013: 앱 프로세스 runtime crate(`stellar-runtime`)와 thread 모델

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §8은 네이티브 셸이 안정 C ABI를 거쳐 Rust 코어에 도달하고, 코어가 문서·command
엔진, binding, layout, scene snapshot, project store 등을 구성하는 구조를 정의합니다. §8.2는 ABI에
Rust 내부 타입을 노출하지 않고 handle·POD·버전형 메시지와 snapshot·command 단위 전달을 쓰도록
요구합니다.

§9.2는 앱 내부 thread를 Native UI, Document(유일한 문서 작성자, command 순차 적용, undo/redo),
Render(불변 scene snapshot 읽기), Worker Pool로 나누고 "문서 모델에 대한 다중 스레드 동시 쓰기는
금지"합니다. §9.3은 render thread가 잠금 없이 읽고 document thread가 렌더 완료를 기다리지 않도록
요구합니다. 구현 계획 XII.1(XII.1.1~XII.1.6)은 이 thread 구성과 종료 순서를 구현 대상으로 둡니다.

기존 crate 정책(II.2.8, `tools/xtask/src/crate_graph.rs`)은 `stellar-ffi`가 `stellar-core-types`에만
의존하도록 허용합니다. 따라서 셸 경계에서 문서·프로젝트 작업에 도달할 경로가 없고, thread 모델을
소유할 crate도 없습니다.

## Decision

1. **새 crate `stellar-runtime`(`crates/runtime`)** 이 앱 프로세스 runtime을 소유합니다. §9.2의
   thread 구성(document thread event loop, 이후 render thread·worker pool)과 thread별 종료 순서
   (XII.1.6)를 이 crate에 둡니다. 외부 의존성 없이 표준 라이브러리 thread·동기화만 사용합니다.
2. **crate 경계(허용 직접 의존)**:
   - `stellar-runtime` → `stellar-core-types`, `stellar-document`, `stellar-commands`,
     `stellar-history`, `stellar-project`, `stellar-storage`, `stellar-assets`,
     `stellar-renderer-core`.
   - `stellar-ffi` → `stellar-core-types`, `stellar-runtime`. ffi는 runtime을 우회해
     document·project에 직접 의존하지 않습니다(단일 작성자 규칙을 셸 경계에서도 유지).
   - 역방향 금지: runtime은 ffi에 의존하지 않고, runtime 아래 crate는 runtime을 참조하지
     않습니다. 셸 입력 타입(`stellar-ffi::input`)의 변환은 ffi 쪽에서 합니다.
   - 실제 의존은 필요할 때만 추가합니다. XII.1.1 시점의 실제 의존은 core-types, document,
     renderer-core(thread 역할)입니다.
3. **document thread = 유일한 작성자(XII.1.1)**:
   - 열린 문서 집합은 document thread 안에서 만들어지고 밖으로 나가지 않습니다. 다른 thread는
     job(`FnOnce(&mut DocumentContext) -> T + Send + 'static`)을 보내기만 하고, job은 document
     thread에서 접수 순서대로 하나씩 실행되며 결과는 소유 값으로만 돌아옵니다.
   - `DocumentContext`와 문서 변경 view `DocumentMut`는 `!Send`·`!Sync`이고 공개 생성자가
     없습니다. 공개 API는 `&mut Document`를 내보내지 않습니다. 성공한 변경마다 문서별
     `DocumentRevision`을 1 올립니다(§9.2 "문서 세대"와 이후 worker 결과 검증의 기준).
   - runtime이 만드는 thread에는 `stellar-renderer-core`의 `ThreadRole`(XI.7.1)을 bind합니다.
     document thread에서 job 결과를 blocking으로 기다리는 호출은 거부합니다(자기 대기 deadlock).
4. **UI→document 수신함(XII.1.2)**: 수신함은 우선순위별 bounded lane(`Input` → `Command` →
   `Background` 순으로 처리, lane 안 FIFO)입니다.
   - 셸 입력 이벤트는 모두 `Input` 우선순위입니다. 절대 상태인 pointer move(장치별)와 resize는
     같은 lane의 마지막 항목이 같은 key일 때만 최신 값으로 대체하고, 상대량인 wheel과 이산
     이벤트(pointer down/up/cancel, key, text composition, focus)는 병합하지 않습니다. 셸 경계는
     공통 `InputEvent`를 `InputEventClass`로 바꿔 접수 옵션을 얻습니다.
   - lane이 차면 `Input`·`Command`는 즉시 `QueueFull`로 거부하고(이미 접수된 항목을 버리지
     않음), `Background`는 가장 오래된 항목을 밀어냅니다. 대체·밀려난 job의 ticket에는 이유를
     알립니다.
   - UI thread의 접수 API는 용량·job 실행을 기다리지 않습니다. `ThreadRole::Ui` thread에서
     job 결과를 blocking으로 기다리는 호출은 거부하고, UI는 대기 없는 확인만 씁니다.
5. **종료**: document thread 종료는 접수 닫기 → 이미 접수된 job 실행 → join 순서입니다. 전체 종료
   순서는 계획 XII.1.6대로 UI stop → command drain/cancel → render stop → worker stop → runtime
   destroy로 고정하며, render thread·worker pool이 생기는 시점에 이 crate에서 구현합니다.

## Alternatives

- **thread 모델을 `stellar-ffi`에 구현**: ABI crate가 문서·프로젝트·렌더 crate 전부에 의존하게 되고,
  ABI 안정성 경계와 runtime 수명 관리가 한 crate에 섞입니다. 채택하지 않습니다.
- **`stellar-project`에 runtime을 둠**: project는 폴더·매니페스트·저장 계층이며 렌더러·thread 역할
  의존이 생기면 계층이 뒤집힙니다. 채택하지 않습니다.
- **`Mutex<Document>`/`RwLock`으로 여러 thread가 공유**: 단일 작성자 규칙(§9.2)을 타입으로 보장하지
  못하고, render thread의 무잠금 읽기(§9.3)와도 맞지 않습니다. 채택하지 않습니다.
- **외부 channel crate 사용**: `config/dependencies.yaml` 승인 목록에 없고 표준 라이브러리로
  충분합니다. 채택하지 않습니다.
- **용량이 차면 UI가 자리가 날 때까지 대기(`std::sync::mpsc::sync_channel`의 `send`)**: document
  thread가 긴 job을 실행하는 동안 UI thread가 멈춥니다(XII.1.2 "blocking UI call 없음" 위반).
  채택하지 않습니다.
- **단일 FIFO에 우선순위 없이 접수**: 백그라운드 요청이 쌓이면 입력 처리가 뒤로 밀립니다(XII.1.2
  "입력 이벤트에 priority"). 채택하지 않습니다.
- **pointer move를 lane 어디서든 같은 key와 병합**: 사이에 낀 down/up과의 순서가 바뀌어 drag 의미가
  깨집니다. 연속된 같은 key만 병합합니다.

## Consequences

- 긍정: 셸 → ffi → runtime → 도메인 crate의 단방향 경로가 생기고, 단일 작성자 규칙이 compile 단계에서
  강제됩니다(compile_fail doctest).
- 긍정: thread 역할 표시가 runtime thread에 일관되게 적용되어 worker 전용 작업 guard(XI.7.1)가
  document thread에서도 동작합니다.
- 부정: 문서 변경 API가 늘어날 때마다 `DocumentMut`(또는 command 적용 경로)에 forwarding이
  필요합니다.
- 부정: 사람용 의존 문서 `docs/architecture/crate-dependency-graph.md`는 이 ADR과 같은 commit에서
  갱신하지 않았으므로 후속 문서 변경으로 맞춰야 합니다(강제 기준은 `crate_graph.rs`).

## Compatibility

파일 포맷·저장 구조에 영향이 없습니다. C ABI(`stellar-ffi`)와 생성 헤더는 이 결정으로 바뀌지
않습니다. ffi가 runtime을 연결할 때 기존 registry handle 규칙(정수 handle, handle별 last error)을
유지합니다.

## Performance

성능 예산(`config/performance-budgets.yaml`)은 바꾸지 않습니다. job 수신함 임계 구역은 접수·꺼내기
한 번(O(1))이며 job 실행·폐기는 lock 밖에서 합니다. lane 용량 기본값은 코드 상수로 둔 초기값이며,
입력 queue latency 측정(XII.3.5) 결과로 조정합니다. document thread가 render 제출을 기다리지 않는
계약은 XII.1.3에서 검증합니다.

## Security

새 외부 의존성과 `unsafe` 코드가 없습니다. job panic은 test profile에서 document thread를 종료시키고
남은 ticket을 실패로 알리며, release profile은 panic 정책(II.1.3)대로 abort합니다.

## Migration

기존 데이터 migration은 없습니다. 기존 crate 정책에 `stellar-runtime` 항목과 `stellar-ffi` →
`stellar-runtime` edge를 추가했으며, 다른 crate의 허용 edge는 바뀌지 않습니다.
