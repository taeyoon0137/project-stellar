# 작업 보고서: IX.11.18 modifier stack CSS 고정 순서 선형화 계획

- WBS: IX.11.18
- 작업명: modifier stack을 CSS 고정 적용 순서로 선형화하는 export 계획 함수
- 상태: 부분 완료 (`[~]`) — 계획 함수와 규칙별 fixture는 끝났습니다. 계획을 읽는 출력기가 없어 실제 DOM 배치(flex item Frame overlay의 containing block) 확인이 남았습니다.
- 결정: 속성 모델 3장·9.1절·9.2절, ADR-0024
- 선행: IX.11.2, IX.10.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/codegen/src/modifier_plan.rs`(신규), `crates/codegen/src/lib.rs`(모듈 등록).

## 구현 요약

- 입력은 문서 crate에 의존하지 않는 지역 타입입니다(`PlanInput`). 출력은 바깥 → 안 순서의 요소 목록,
  굽기 지점, 래스터화 여부, 진단입니다.
- 규칙은 다음과 같습니다.
  - CSS 단계가 단조인 가장 긴 앞 구간을 node 요소에 담습니다.
  - 단일 값 단계(clip·mask·opacity·blend)의 반복이나 역행 지점마다 바깥 wrapper를 만들고 `approximated-structure`로
    진단합니다.
  - Shadow·Blur는 filter 목록 하나로 합칩니다. Frame Shadow는 filter 앞에서만 `box-shadow`입니다.
  - Boolean·StrokeExpand·Corner까지는 path로 굽습니다(Shape). Frame 등은 래스터화합니다.
  - border가 있는 Frame의 Fill·Stroke는 overlay에 칠합니다(음수 inset, 같은 radius). 자기 칠하기가 있는 Path clip
    Frame은 clip wrapper로 나눕니다.
  - stacking 선언: absolute는 `z-index: 0`, 그 밖은 `position: relative; z-index: 0`.
  - Transform modifier는 `translate(-o) matrix() translate(o)`로 씁니다. Blend가 있으면 합성 범위 진단을 냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(47). 순서별 구조, 같은 stack 같은 구조, 굽기 지점, stacking 선언, overlay와 radius, Path clip 분리, transform-origin 보정, Blend 진단 fixture. |

## 차이(작업 지시와 다른 점)

- 값의 세부(색·반경 수치)는 출력기가 채우는 자리 표시(`<fill>` 등)로 둡니다. 실제 값 표기는 XXII.2 출력기가 CSS
  값 formatter로 씁니다.
- "flex item인 Stack Frame overlay의 containing block"은 stacking 규칙상 모든 Frame이 positioned라는 점으로
  보장합니다. 실제 DOM 배치 fixture는 출력기(XXII.2)가 생길 때 더합니다.

## 남은 위험 / 한계

- 이 계획을 읽는 출력기가 아직 없습니다.
