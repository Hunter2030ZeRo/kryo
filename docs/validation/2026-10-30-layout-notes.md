# 현재 구현과 구조 이동

기준: 2026-10-30. 이 기록은 당시 스냅샷이며 이후 기능 구현은 따로 갱신해야 한다.

## 현재 구현

- `_start → init_gdt → init_idt → A → int3 → handler의 B → C → cli/hlt`.
- GDT의 null/code/data 항목, GDTR 로드 및 CS/SS 갱신.
- `spin::Once`로 IDT를 만들고 breakpoint handler 등록.
- TSS 파일은 16 KiB 정렬 스택 선언까지 작성 중이다. TSS 객체·TR·IST·double fault는 아직 연결하지 않았다.
- core/interfaces 라이브러리, 사용자 서비스, ICE, allocator·scheduler·IPC 등 예약 영역의 기능은 미구현이다.

## 이동 및 변경 범위

| 이전 | 현재 | 내용 변경 |
| --- | --- | --- |
| `src/main.rs` | `kernel/image/src/main.rs` | HAL을 import하도록 연결 변경, 불필요해진 crate feature 선언 이동. 실행 본문 보존 |
| `src/arch/` | `hal/src/arch/` | `init_gdt`, `init_idt` 두 함수만 HAL 재노출을 위해 `pub`로 변경 |
| `src/debug/` | `hal/src/debug/` | 바이트 그대로, 이전처럼 모듈 연결 없음 |
| `linker.ld`, `limine.conf` | `boot/` 아래 | 바이트 그대로 |
| root `Cargo.toml` | workspace 및 구성원별 manifest | 기존 외부 dependency 버전·profile 보존. `kryo` 바이너리 이름 유지 |
| `.cargo/config.toml`의 링커 인자 | `kernel/image/build.rs` | 커널 바이너리에만 적용, manifest 기준 경로 사용 |
| `Makefile` | 같은 위치 | boot 설정 경로·package 지정·target/profile 연결. release 선택이 실제 빌드에도 적용되도록 정리 |

`buddy_system_allocator`와 `talc`는 core의 후보 의존성으로 남겼다. crate를 가져오는 것과 실제 allocator를 설치하는 것은 다르다.
사용자가 편집하던 TSS 파일, 링커 스크립트, Limine 설정의 내용은 보존했다. 기존 target/build 산출물은 검증 때 덮어쓰지 않는다.

## 검증

실제 `/home/entity27th/kryo`의 이동 후 소스를 대상으로 확인했다. `CARGO_TARGET_DIR`, `BUILD_DIR`, `KERNEL`을 임시 검증 경로로 지정해 기존 산출물과 로그를 보존했다.

| 검증 | 결과 |
| --- | --- |
| `cargo check --workspace --locked --offline` | 성공 |
| `make iso` | debug 빌드와 ISO 생성 성공 |
| `make iso PROFILE=release` | release 빌드와 ISO 생성 성공 |
| 두 ISO의 QEMU TCG 부팅 | 각각 정확한 `ABC` |
| CPU 상태 | 각각 `CS=0x0008`, `SS=0x0010`, GDT base와 해당 ELF symbol 일치, GDT limit `0x17`, IDT limit `0xfff` |
| 마지막 실행 상태 | 각각 `HLT=1`, IF=0 |
| 원본 보존 검사 | 작성 중인 TSS 등 기존 코드 내용, linker/limine 설정, 외부 dependency 버전·checksum, 기존 build 산출물 보존 |
| 문서·구성 | 상대 링크 유효, Cargo 구성원 네 개 확인 |

상세 레지스터·명령·소스 해시는 [검증 기록](2026-10-30-layout.json)에 보관했다.
미완성 TSS의 unused import/dead code 경고와 아직 사용하지 않는 의존성 경고는 남아 있다. 경고를 숨기거나 TSS를 임의로 완성하지 않았다.
이 결과는 기존 부팅 흐름의 구조 이동 회귀 검증이며 TSS/IST, Ring 3, IPC, 드라이버 및 실제 하드웨어 성능의 성공 증거가 아니다.

이동 전 소스·설정 백업: `/home/entity27th/문서/ChatGPT/프로젝트 Kryo/backups/kryo-before-layout-2026-10-30.tar.gz`.
Git commit/push는 수행하지 않았다.
