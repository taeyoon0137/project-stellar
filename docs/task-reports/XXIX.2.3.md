# 작업 보고서: XXIX.2.3 구독 CLI 실행 대상 지정

- WBS: XXIX.2.3
- 작업명: 사용자가 확인한, 수정하지 않은 공식 CLI만 실행 대상으로 삼는 신원 확인
- 상태: `[x]` 완료
- 선행: XXIX.2.2
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/target.rs`(신규): `ConfirmedTarget`, `FileFingerprint`, `confirm_target`, `verify_target`, `CodeVerifier`, `AppleStaticCodeVerifier`
- `crates/agent-host/tests/target_identity.rs`

## 구현 요약

- **사용자 확인 기록**: 사용자가 고른 경로의 정규 경로(symlink 해석 뒤), CLI 종류, 파일 지문(장치·inode·크기·수정 시각·mode). 기록 저장 위치(OS 사용자 데이터)는 XXIX.3.5가 정합니다.
- **같은 파일 확인**: 실행 전 경로를 다시 정규화해 기록과 같은지, 지문이 같은지 봅니다. 업데이트 등으로 바뀌면 `ChangedSinceConfirmation`(다시 확인 필요). 서명 확인 뒤에도 지문을 다시 봅니다.
- **파일 조건**: 일반 파일, 실행 bit, group·other 쓰기 불가, 소유자 root 또는 현재 사용자.
- **신원(Apple 공식 Code Signing Services)**: `SecStaticCodeCreateWithPath` + `SecStaticCodeCheckValidity`, flag `kSecCSStrictValidate | kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSNoNetworkAccess`. 요구 사항은 profile Team ID의 `anchor apple generic and certificate leaf[subject.OU] = "<team>"`(TN3127 "Inside Code Signing: Requirements", local-ipc `CodeRule::SameTeam`과 같은 문자열). strict 검증은 실행 파일 내용을 서명된 code directory hash와 비교하므로 수정된 파일은 실패합니다. `kSecCSNoNetworkAccess`로 확인 중 network를 쓰지 않습니다.
- **CLI 공식 문서 근거**: Claude Code CLI reference는 `claude install`(native binary 설치)과 `claude update`를 사용자 명령으로 문서화합니다. Stellar는 이 명령들을 실행하지 않고, 실행 파일을 설치·업데이트·수정·래핑하지 않습니다(쓰기로 여는 코드 0). 서명 없는 배포 형태(해석기가 실행하는 script)는 신원을 확인할 수 없어 실행 대상이 아닙니다.
- 신원 확인이 끝나기 전에는 CLI를 한 번도 실행하지 않습니다(버전 확인도 그 뒤).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| 신원 확인 실패 fixture 5종(서명 없음, Team 불일치, 실제 Apple 확인 수단으로 대역 확인, CLI 종류 불일치, profile Team ID 없음): 실행 0(어느 단계 mark도 없음) | 통과 |
| 확인 중 network 호출 0: `kSecCSNoNetworkAccess`, crate network API inventory 0 | 통과 |
| CLI 실행 파일 쓰기 0: 확인·세션 전후 실행 파일 bytes 동일 | 통과 |
| 확인 뒤 내용이 바뀐 파일은 서명 확인 전에 거부(검증 호출 0) | 통과 |
| symlink 교체는 정규 경로 기록이라 실행 대상을 바꾸지 못함 | 통과 |
| 권한 0777·디렉터리·실행 bit 없는 파일은 확인 단계에서 거부 | 통과 |
| `AppleStaticCodeVerifier`: `/bin/ls`(Apple 서명)는 다른 Team 요구 사항 실패, 서명 없는 script·없는 경로 실패 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 공급자별 실제 Team ID는 확인하지 못해 공식 profile의 `team_id`를 null로 두었습니다. 그래서 공식 profile은 실행되지 않습니다(XXIX.2.6).

## 남은 위험 / 한계

- 공급자 서명 Team ID 값 확인은 profile 활성화(제품 소유자 약관 검토 뒤)와 함께 해야 합니다.
- 확인 기록의 저장과 사용자 확인 UI는 XXIX.3.5·XXIX.4.8 범위입니다.
