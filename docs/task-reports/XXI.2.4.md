# 작업 보고서: XXI.2.4 native app UI·canvas 경로 WebView 0 inventory test

- WBS: XXI.2.4
- 작업명: native app UI와 canvas rendering path에 WebView를 사용하지 않는 inventory test 유지
- 상태: 부분 완료 (`[~]`) — inventory test는 있고 통과합니다. 선행 XXI.2.1(WKWebView 생성)·XXI.2.2(Windows, 보류)가 끝나지 않아
  plugin panel 허용 위치를 실제 panel 파일로 좁히는 일이 남았습니다.
- 선행: XXI.2.1, XXI.2.2
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md), 기술 아키텍처 §5(본체 UI는 WebView 기반이 아님)

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/tests/webview_inventory.rs`(신규).

## 구현 요약

- `apps/`, `crates/`, `tools/`의 Swift·Objective-C·Rust·Cargo manifest·Xcode project·plist와 루트 `Cargo.toml`에서 WebView 표지
  (`WKWebView`, `import WebKit`, `WebKit.framework`, `WebView2`, 식별자 경계의 `wry`·`webview`)를 찾습니다.
- 허용 위치는 plugin panel host인 `crates/plugin-host/`뿐입니다. CSS 값 `-webkit-` 같은 문자열은 세지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test webview_inventory` | 2 passed(plugin panel 밖 WebView 0) |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 문자열 검색이라 다른 이름으로 WebView를 감싸면 놓칩니다. WebView 연결 때 Xcode target의 link framework 검사(WebKit)를 더하는 편이 확실합니다.
