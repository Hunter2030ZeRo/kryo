# 검증 영역

하위 디렉터리는 시나리오의 자리이며 아직 테스트 구현이 아니다. 부팅과 CPU 상태는 QEMU에서, 안전한 순수 로직은 host에서, 최종 성능·전력·장치 복구는 실제 하드웨어에서 구분해 검증한다.
현재 자동 검증은 Cargo check/build, 수동 실행은 루트의 `make run`을 사용한다. 루트 `tests/`는 virtual workspace이므로 Rust 테스트가 자동 등록되는 위치가 아니다. 구현할 때 해당 package 또는 별도 host harness에 연결한다.
