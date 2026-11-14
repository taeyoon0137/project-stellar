# 차단 보고서: XIV.6.2 초기 index 적용 경합 (이력)

## WBS

- WBS: XIV.6.2
- 작업명: 실제 index 적용 전 경합·REUC 보존 검토
- 상태: `[!]` — commit `39bf1612` 당시의 차단 기록. 현재 해소 범위는 [완료 보고서](XIV.6.2.md)를 따릅니다.
- 선행: XIV.6.1 (`[x]`)

## 실패 선행조건

- 사전 snapshot 검사와 Git 잠금 취득 사이에 외부 writer가 변경하면 단순 index-info는 같은 경로를 덮었습니다.
- `read-tree -i -m base candidate`도 D/F 경합과 REUC를 보존하지 않았습니다. 직접 `.git` 쓰기로 우회하지 않았습니다.

## 재현 명령

당시 Apple Git 2.54.0에서 임시 GitFixture와 `cargo test -p stellar-git-porcelain index_publish_probe:: -- --nocapture`를 사용했습니다. 처음 probe 기대 오류로 101, 관찰 분기 수정 후 3개 통과(위험 재현)였습니다. 임시 probe는 제거했고 전체 과거 코드는 commit `39bf1612`에 있습니다.

| 핵심 예시 | 관찰 |
|---|---|
| snapshot → 외부 same-path 변경 → index-info | 외부 OID를 candidate로 덮음 |
| 후보 d/a, 외부 d → read-tree merge | 외부 d 소실 |
| 후보 d, 외부 d/a → read-tree merge | exit 0에도 후보 d 미적용 |
| unrelated REUC 3개 → read-tree merge | REUC 0개로 감소 |

현재 실행 가능한 회귀는 `cargo test -p stellar-git-porcelain index_publish::`에 있습니다. [update-index 소스](https://github.com/git/git/blob/v2.54.0/builtin/update-index.c#L1047-L1055)의 잠금 실패 처리 때문에 lock 파일 존재만으로 소유를 판단해서는 안 됩니다.

## 필요한 결정과 환경

- 기존 typed/gated runner 안에서 자식의 실제 잠금 소유를 증명하고 잠금 안에서 비교해야 합니다. 현재 macOS 구현과 Apple/upstream 지원 범위는 완료 보고서에 기록했습니다.
- GNU/Linux·다른 버전·index 상태는 별도 실측 없이 지원으로 확대하지 않습니다.

## 변경되지 않은 파일

- 당시 production stage API·runtime stage 명령·FFI/UI는 추가하지 않았습니다. 이후 변경은 완료 보고서와 해당 Task 커밋을 따릅니다.
- 선행 부분 선택·후보 직렬화·blob 구현은 그대로 재사용합니다.

## 재개 조건

- cold start·같은 경로·D/F·flags·REUC·외부 writer를 기존 gated 경로에서 검증할 것.
- 실제 stage 뒤 Index provider의 semantic 결과와 Working Tree bytes·메모리 revision을 확인할 것.
- 결과 소유가 불명확한 실패를 무조건 복원하지 말고 진단할 것. 후속 복원·semantic 실패 처리는 단계별 증거로 완료 판정합니다.
