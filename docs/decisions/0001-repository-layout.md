# 0001: 계층별 디렉터리와 네 개의 Cargo 구성원

상태: 2026-10-30 구조 작업에서 적용.

사용자가 임시 레이아웃의 반복 이동 대신, 직접 채워 나갈 장기 구조를 요청했다.
이전의 Ring 3 드라이버/파일시스템 정정을 반영해 `boot / hal / interfaces / kernel / userspace / compat / tests / tools / docs`로 역할을 나눈다.

- 실제 crate는 HAL, interfaces, kernel core, kernel image 네 개다. 기존 구현은 HAL과 image로 이동한다.
- 비어 있는 기능은 디렉터리만 예약한다. 아직 약속하지 않은 Rust API나 syscall 번호를 만들어 넣지 않는다.
- kernel core는 장치 드라이버·VFS·네트워크 스택·ICE·BetAFS 구현에 의존하지 않는다.
- 후속 BetAFS는 외부 저장 엔진이다. 사용자 서비스의 연결 위치만 둔다.
- 새 기능은 기존 경계 아래에 추가한다. 최상위 구조를 바꿔야 할 때는 이유와 의존성 변화를 새 결정 문서에 남긴다.
- 커널 기능 구현은 사용자가 담당한다. 구조를 만든 요청을 다른 기능의 자동 구현 승인으로 확장하지 않는다.
