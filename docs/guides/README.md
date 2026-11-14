# 개발 가이드

이 폴더는 현재 코드가 실제로 어떻게 동작하고 어떻게 나뉘어 있는지 설명하는 안내 문서를 둡니다. 각
문서는 작성 기준 커밋을 밝히고, 코드에서 확인한 내용과 아직 구현되지 않은 계획을 구분해 씁니다.

이 폴더의 문서는 기술 기준이 아닙니다. 구현 판단의 1차 기준은
[기술 아키텍처 정의서](../PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md)이고, 결정 기록은 [ADR](../adr/),
작업 범위는 [구현 계획서](../PROJECT_STELLAR_IMPLEMENTATION_PLAN.md)에 있습니다. 안내 문서와 코드가 다르면
코드가 기준이며, 코드를 바꾸는 작업은 관련 안내 문서도 함께 고칩니다.

## 문서 목록

| 문서 | 내용 | 기준 커밋 |
|---|---|---|
| [stellar-cli.md](stellar-cli.md) | headless `stellar` 명령의 현재 명령·인자·JSON 결과 envelope·종료 코드, 실제 실행 예시, ADR-0005 L1 계획 | `993d7d0` |
| [crate-map.md](crate-map.md) | workspace member 32개의 책임, 주요 module, 허용 의존(crate graph 정책)과 실제 의존, 주요 ADR | `993d7d0` |

## 관련 문서

- [Crate 의존 그래프](../architecture/crate-dependency-graph.md)
- [ADR-0005: 3계층 에이전트 도구 구조](../adr/ADR-0005-agent-tool-layers.md)
- [Stellar 파일 포맷](../spec/stellar-format.md)
