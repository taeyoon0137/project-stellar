# 작업 보고서: XI.5.2 generational GPU handle

- WBS: XI.5.2
- 작업명: GPU handle을 generational ID로 감싸고 backend native pointer를 공통 scene에 노출하지 않음
- 상태: 완료 (`[x]`)
- 선행: III.1.3 (`[x]`), XI.5.1 (이 lane에서 `11a70c9`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/gpu_handle.rs`(신규) — `GpuHandle<K>`, `BufferHandle`, `TextureHandle`,
  `HandleKind`(sealed), `BufferKind`, `TextureKind`, `HandleTable<K, T>`, `HandleError`.
- `crates/renderer-core/src/backend/mod.rs` — XI.5.1의 opaque `BufferHandle(u64)`/`TextureHandle(u64)`를
  generational handle 재노출로 바꿈, `From<HandleError> for BackendError`, 모듈 문서 handle 절.
- `crates/renderer-core/src/backend/mock.rs` — mock native 객체를 `HandleTable`에 보관, 재 init 시
  `invalidate_all`, buffer 범위 검사.
- `crates/renderer-core/src/backend/tests.rs` — raw handle 생성 제거, backend 경유 stale handle 테스트.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.5.2.md`(신규), 계획서(XI.5.2 `[ ]`→`[x]`).

## 구현 요약

- **handle 구성**: `GpuHandle<K> { index: u32, generation: NonZeroU32, epoch: u32 }` 12 bytes입니다.
  native pointer·객체를 담지 않고, `Option<handle>`도 12 bytes입니다(generation niche). 공개 생성자가
  없어 `HandleTable`만 발급합니다(compile_fail doctest). `BufferKind`·`TextureKind` 표식으로 종류를
  타입 수준에서 구분합니다(buffer → texture 대입 compile_fail doctest). 직렬화를 제공하지 않아
  저장·FFI에 나가지 않습니다(III.1.3 `RuntimeEntityId`와 같은 원칙). renderer-core는 crate 의존
  정책상 core-types를 직접 쓸 수 없어 같은 slot·generation 방식을 이 crate에 두었습니다.
- **HandleTable**: backend가 native 객체(T)를 보관합니다. `insert`는 빈 slot을 재사용하고, `remove`는
  generation을 올려 원본·복사본 handle을 모두 stale로 만듭니다. generation이 넘칠 slot은 은퇴시켜
  다시 쓰지 않습니다. `invalidate_all`(device lost)은 모든 값을 돌려주고 epoch를 올려 이전 device의
  handle을 `WrongEpoch`로 만듭니다. epoch가 넘치면 거부하고 table을 바꾸지 않습니다.
- **backend 연결**: `HandleError`는 `BackendErrorKind::InvalidUsage`(공간 소진은 `OutOfHostMemory`)와
  정적 detail로 바뀌어 trait 경계를 넘습니다. scene은 GPU handle을 담지 않고 `SceneResourceRef` 값
  식별자만 씁니다(XI.1.1 그대로).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 57 passed, 0 failed; doctest 3 passed(compile_fail 3) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 파괴 후 원본·복사본 handle 조회·재파괴 실패, 같은 slot 재사용 시에도 이전 handle은 새 값을 가리키지 않음, 다른 table의 범위 밖 handle 실패(`stale_handle_fixture_fails`) | 통과 |
| device reset 후 이전 handle 전부 `WrongEpoch`, 같은 index·generation의 새 handle과 구분, epoch 넘침 거부(`device_reset_invalidates_every_handle`) | 통과 |
| generation 넘침 slot 은퇴(`exhausted_generation_retires_the_slot`) | 통과 |
| handle 12 bytes·native pointer 없음, `Option` 크기 동일, Debug 형식(`handles_carry_no_native_pointer`) | 통과 |
| backend trait 경유: 파괴된 texture handle 갱신·재파괴 실패, 다른 device handle 실패, 재 init 뒤 이전 buffer·texture handle 실패, 새 handle은 성공(`stale_handles_fail_through_the_backend`) | 통과 |
| 공개 생성자 없음·종류 혼용 불가(compile_fail doctest 2건) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 III.1.3은 명시 row-ID이고 `[x]`입니다. 그 타입(`RuntimeEntityId`)은 core-types에 있어 renderer-core가
  직접 쓸 수 없으므로(II.2.8) 같은 원칙의 별도 타입을 정의했습니다. 완화 적용은 없습니다.
- XI.5.1에서 정의한 opaque handle의 `from_raw`/`raw`는 제거했습니다(같은 lane, 후속 사용처 없음).

## 남은 위험 / 한계

- handle은 backend 인스턴스 안에서만 의미가 있습니다. 여러 backend 인스턴스가 같은 epoch로 table을
  만들면 서로의 handle을 구분하지 못합니다. backend는 인스턴스마다 다른 epoch로 시작해야 합니다
  (Metal 구현 XI.8에서 적용).
- GPU가 아직 쓰는 resource의 해제 지연은 XI.5.3(frame-local allocator·지연 해제)에서 다룹니다.
