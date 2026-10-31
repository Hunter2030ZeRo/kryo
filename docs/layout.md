# 디렉터리 지도

최상위 경계는 유지하고 기능을 아래에 추가한다. 예약 디렉터리는 `.gitkeep`으로 보존한다.
디렉터리를 만든 것만으로 Rust 모듈이 되지는 않는다. 구현한 뒤 부모의 `mod` 선언과 필요한 공개 범위를 추가한다.
Ring 3 구성원은 실행 환경이 정해진 뒤 별도 Cargo package로 등록한다.

## 현재 코드의 위치

| 경로 | 상태·책임 |
| --- | --- |
| `hal/src/arch/x86_64/gdt.rs` | 기존 GDT와 CS/SS 로드. TSS descriptor 연결은 다음 단계. |
| `hal/src/arch/x86_64/tss.rs` | 사용자가 작성 중인 16 KiB 정렬 스택 선언. 내용 그대로 이동. |
| `hal/src/arch/x86_64/interrupts/idt.rs` | 기존 spin::Once IDT와 int3 핸들러. |
| `hal/src/arch/x86_64/interrupts/traps.rs` | 기존 빈 예외 처리 파일. |
| `hal/src/arch/x86_64/interrupts/fred.rs` | 기존 빈 FRED 파일. 선택적 후속 구현. |
| `hal/src/arch/x86_64/cpu.rs` | 기존 빈 CPU 파일. |
| `hal/src/debug/` | 기존 빈 debug 파일. 현재 Rust 모듈 트리에는 연결하지 않음. |
| `kernel/image/src/main.rs` | 기존 _start/panic/ABC 흐름. 함수 가져오기 경로만 HAL crate로 변경. |
| `kernel/image/build.rs` | 이 바이너리에만 boot/linker.ld를 절대 경로로 전달. |
| `boot/limine.conf, boot/linker.ld` | 기존 파일의 위치만 변경, 내용 보존. |

## 예약한 기능 영역

다음은 모두 아직 구현되지 않은 위치다.

### 부팅과 하드웨어

| 경로 | 책임 |
| --- | --- |
| `boot/initramfs/` | 초기 Ring 3 프로그램·서비스 manifest를 담는 부트 이미지 입력. 런타임 파일시스템에 의존하지 않는 bootstrap. |
| `hal/src/arch/x86_64/boot/` | CPU 진입 직후의 아키텍처별 준비. Limine 응답 해석은 image에서 수행. |
| `hal/src/arch/x86_64/apic/` | 로컬 APIC·I/O APIC 하드웨어 제어와 타이머 전달. |
| `hal/src/arch/x86_64/paging/` | 페이지 테이블·TLB 하드웨어 조작. 매핑 정책은 core/mm. |
| `hal/src/arch/x86_64/context/` | 레지스터 저장·복구, 커널/사용자 모드 전환. |
| `hal/src/arch/x86_64/syscall/` | 시스템 호출 진입·복귀와 레지스터 규약. 호출 의미론은 core/syscall. |
| `hal/src/arch/x86_64/smp/` | AP 시작과 per-CPU 기반. GDT/TSS/스택은 CPU별 소유권을 갖도록 확장. |
| `hal/src/arch/x86_64/iommu/` | DMA 격리 하드웨어 조작. 장치 제어 프로토콜은 Ring 3. |
| `hal/src/arch/x86_64/asm/` | Rust inline asm으로 표현하기 어려운 진입·복귀 조립 코드. |
| `hal/src/io/` | 권한 있는 MMIO/port I/O 저수준 연산. 범용 장치 드라이버를 넣지 않음. |

### 공용 계약

| 경로 | 책임 |
| --- | --- |
| `interfaces/src/syscall/` | 네이티브 syscall 번호·인자·오류 계약. Linux/NT 번호를 혼합하지 않음. |
| `interfaces/src/handle/` | 핸들 표현과 접근 권한 비트. 실제 객체 테이블은 kernel/core. |
| `interfaces/src/ipc/` | 작은 메시지·공유 큐의 wire layout, 버전, 메모리 순서 계약. |
| `interfaces/src/memory/` | 공유 메모리·매핑·페이저 요청 계약. |
| `interfaces/src/device/` | IRQ·MMIO·DMA 권한 및 Ring 3 장치 서비스 프로토콜. |
| `interfaces/src/filesystem/` | VFS·파일시스템·블록 서비스 사이의 요청/완료 계약. |
| `interfaces/src/service/` | 서비스 식별자·세대·종료·재연결·교체 계약. |

### 커널 기구

| 경로 | 책임 |
| --- | --- |
| `kernel/core/src/init/` | 아키텍처 독립적인 자원 초기화와 bootstrap 과정. |
| `kernel/core/src/mm/frame/` | 물리 프레임의 할당·반납과 소유권. |
| `kernel/core/src/mm/vm/` | 주소 공간·매핑 권한·페이지 폴트·사용자 페이저 연결. |
| `kernel/core/src/mm/heap/` | 커널 힙. buddy/talc의 실제 역할·선택은 아직 미정. |
| `kernel/core/src/mm/shared/` | 공유 메모리 객체·등록 버퍼·대여 수명. |
| `kernel/core/src/mm/dma/` | 핀된 메모리·DMA 매핑·권한 회수. IOMMU 하드웨어는 HAL. |
| `kernel/core/src/task/` | 프로세스·스레드·수명·종료·격리된 주소 공간. |
| `kernel/core/src/sched/` | 실행 큐·선점·대기·깨우기. 초기 구현은 단일 CPU. |
| `kernel/core/src/object/` | 커널 객체의 식별·참조·수명. VFS 파일 객체나 NT 객체는 포함하지 않음. |
| `kernel/core/src/capability/` | 권한 검증·위임·철회·핸들 테이블. |
| `kernel/core/src/ipc/` | endpoint·작은 메시지·notification·대기/깨우기. 대용량 본문은 공유 메모리. |
| `kernel/core/src/interrupt/` | IRQ 소유권·마스킹·사용자 드라이버 통지. CPU 예외 게이트는 HAL. |
| `kernel/core/src/syscall/` | 네이티브 호출 디스패치·인자 검증·사용자 메모리 접근 경계. |
| `kernel/core/src/sync/` | 커널 내부 동기화·락 순서·원자적 자료구조. |
| `kernel/core/src/time/` | 단조 시간·deadline·대기 타이머. 하드웨어 카운터는 HAL. |
| `kernel/core/src/security/` | 공통 권한 검증 원칙·감사·격리 불변 조건. 서비스 정책은 Ring 3. |
| `kernel/core/src/diagnostics/` | 진단·panic 보고·추적·계측. 출력 장치의 저수준 접근은 HAL. |

### 사용자 공간

| 경로 | 책임 |
| --- | --- |
| `userspace/libsys/` | 네이티브 syscall·IPC의 사용자측 래퍼. 아직 사용자 타깃/ABI가 확정되지 않음. |
| `userspace/runtime/` | 사용자 실행 환경·시작/종료·allocator·executor 연결. 런타임 라이브러리는 미선정. |
| `userspace/init/` | 최초 사용자 프로세스. 초기 capability를 전달받아 서비스를 시작. |
| `userspace/supervisor/` | 서비스 감시·재시작·상태 인계·세대 관리. 감독자 자체 장애도 고려. |
| `userspace/device-manager/` | 장치 발견·드라이버 매칭·권한 배분·교체 정책. |
| `userspace/vfs/` | namespace·mount·파일 핸들·파일시스템 라우팅과 POSIX 지향 의미론. |
| `userspace/pager/` | 파일-backed 페이지 공급·캐시 정책. 커널의 매핑/페이지 폴트 기구와 분리. |
| `userspace/net/` | 네트워크 스택과 소켓 서비스. 패킷 드라이버와 커널 IRQ 기구는 별도. |
| `userspace/drivers/serial/` | Ring 3 직렬 장치 서비스. 초기 debugcon과 구별. |
| `userspace/drivers/virtio/` | QEMU에서 시작할 VirtIO 장치 드라이버. |
| `userspace/drivers/nvme/` | NVMe 장치 서비스와 요청/완료 큐. |
| `userspace/drivers/input/` | 키보드·포인터 등 입력 장치 서비스. |
| `userspace/drivers/display/` | 디스플레이 장치 서비스. |
| `userspace/drivers/network/` | 네트워크 장치 서비스. 프로토콜 스택은 userspace/net. |
| `userspace/filesystems/tmpfs/` | 초기 사용자 공간 메모리 파일시스템. |
| `userspace/filesystems/betafs/` | 독립 BetAFS 저장 엔진을 Kryo 서비스에 연결하는 어댑터 위치. 엔진 소스 복제 금지. |

### 호환성

| 경로 | 책임 |
| --- | --- |
| `compat/ice/linux/` | Linux ABI·프로세스/실행 파일 의미론을 native 계약에 연결. |
| `compat/ice/nt/` | NT ABI·PE·핸들 의미론을 native 계약에 독립 연결. ICE-Linux 위에 얹지 않음. |
| `compat/ice/userspace/` | 호환 사용자 라이브러리·로더·런타임 연결. macOS/XNU는 현재 범위 밖. |

### 검증과 도구

| 경로 | 책임 |
| --- | --- |
| `tests/unit/` | 하드웨어 독립 로직의 host 테스트. 실제 테스트 crate/harness는 구현 시 추가. |
| `tests/boot/` | QEMU 부팅·예외·CPU 상태의 회귀 시나리오. |
| `tests/syscall/` | Ring 3 진입·사용자 포인터·네이티브 호출 검증. |
| `tests/ipc/` | 권한 전달·공유 버퍼·큐·경쟁·대기/깨우기 검증. |
| `tests/recovery/` | 서비스 충돌·교체·오래된 완료·DMA 중단·재연결 시험. |
| `tests/compat/` | ICE Linux/NT 통합 시나리오. |
| `tests/perf/` | IPC 지연·복사량·메모리·깨우기·전력·복구 시간. 실제 하드웨어 수치 별도. |
| `tools/xtask/` | 향후 host 빌드·이미지·검증 도구. 지금은 Makefile을 사용. |
| `tools/debug/` | GDB/QEMU 디버깅 도구와 재현 절차. |

`object`는 수명·식별, `capability`는 권한, `security`는 공통 검증/감사 원칙을 담당한다. 같은 핸들 테이블이나 권한 검사를 중복 구현하지 않는다.
