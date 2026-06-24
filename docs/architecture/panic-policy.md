# Panic 정책 (II.1.3)

## 원칙

- **release 빌드**: `panic = "abort"`. 앱 라이브러리 release 경로에서 unwinding을 사용하지 않습니다.
- **테스트**: unwind를 유지합니다. 테스트는 `profile.test`(= `profile.dev` 상속, panic 미재정의)에서
  실행되므로 `panic = "abort"`의 영향을 받지 않고 `should_panic` 등 unwind 기반 테스트가 동작합니다.
- **FFI 경계**: panic이 FFI 경계를 넘어가지 않아야 합니다. `ffi` crate(II.4.4)는 모든 export 함수에
  `catch_unwind`를 적용해 panic을 `InternalPanic` 오류 코드로 변환합니다. release(`abort`)에서는
  애초에 panic을 발생시키는 프로덕션 경로가 없어야 하며, 이는 별도 테스트로 검증합니다.

## 적용 위치

- 루트 `Cargo.toml`의 `[profile.release]`에 `panic = "abort"`.
- `[profile.dev]`/`[profile.test]`는 panic을 재정의하지 않아 unwind를 유지합니다.

## 주의

- `panic = "abort"`인 profile에서는 test harness가 동작하지 않으므로 `cargo test --release`는
  사용하지 않습니다. 테스트는 dev/test profile(`cargo test`)로 실행합니다.
