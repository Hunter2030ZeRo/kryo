# Kryo

Fast and efficient micro-kernel for Cryolite.

Rust `no_std` x86-64 커널. Limine로 부팅하며 일반 드라이버·파일시스템·네트워크는 Ring 3 서비스로 둔다.

현재 GDT/TSS/IDT, 예외 진단, 부팅 메모리 정규화, 물리 프레임 소유권·할당·반환, BSP 부팅 페이지 매핑을 구현했다. 사용자 프로세스·서비스와 ICE/BetAFS 통합은 아직 구현하지 않았다.

## 문서

- [설계와 계층](docs/architecture.md)
- [소유권·CPU·권한·IPC·복구 계약](docs/design-contract.md)
- [디렉터리 지도](docs/layout.md)
- [현재 상태와 검증 증거](docs/status.md)
- [구현 순서](docs/roadmap.md)
- [개발 규칙](docs/development.md)

Cargo 구성원은 `hal`, `interfaces`, `kernel/core`, `kernel/image` 네 개다. `userspace/`와 `compat/ice/`는 후속 서비스·연결부의 위치다. 예약 디렉터리의 존재가 구현을 뜻하지 않는다.

## 빌드와 검사

Rust nightly, `x86_64-unknown-none`, Limine, xorriso, QEMU, Python 3가 필요하다. Limine 기본 위치는 `/usr/share/limine`이다.

~~~sh
make check
make test
make iso
make run
make iso PROFILE=release
make smoke
python3 tools/boot_smoke.py --all
~~~

`make test`는 Linux 호스트의 자료구조·주소 검사를 실행한다. `make smoke`는 `boot-tests` feature로 별도 ISO를 만들고 실제 QEMU에서 RAM 초기화·128회 매핑/해제·프레임 회수와 ABC를 확인한다. 일반 부팅에는 이 반복 검사가 포함되지 않는다.

기본 커널은 `target/x86_64-unknown-none/debug/kryo`, ISO는 `build/kryo.iso`다. smoke 로그와 증거는 `build/smoke/debug/` 또는 `build/smoke/release/`에 저장해 기존 `build/debug.log`와 분리한다. 검사 실패 시 로그를 보존하고 자동 재시도하지 않는다.

현재 BSP 매핑 검사는 사용자 주소 공간·SMP·실기 성능의 검증이 아니다. 상세한 지원 범위는 [상태 문서](docs/status.md)를 따른다.
