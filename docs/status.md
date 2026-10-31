# 현재 구현 상태

기준: 2026-10-31. 실제 구현과 아래 검증 기록을 기준으로 작성했다.

## 구현된 기능

- Limine의 명시적인 x86-64 4단계 페이징 요청과 부팅 메모리·HHDM·커널 기준 주소 해석.
- GDT/CS/SS, TSS/TR, double-fault IST, IDT, breakpoint 및 GP/PF 비상 진단.
- 물리 구간 검증·정렬, 예약 영역 합집합 차감과 Usable RAM 정규화.
- 비트맵·요약·탐색 힌트 기반 단일 4KiB 프레임 할당과 명시적 반환.
- 복제·위조할 수 없는 OwnedFrame, 한 번만 활성화하는 풀, 다른 풀 반환 거부.
- HHDM을 통한 소유 프레임 초기화와 주소·페이지 경계 검증.
- BSP의 사용하지 않는 PML4 슬롯에 분리된 테이블을 준비한 뒤 연결하는 BootPageSlot.
- supervisor RW+NX leaf의 매핑·해제와 로컬 TLB 정리, 분리 완료 후 테이블 프레임 반환.
- 초기 debugcon 출력, 32bpp RGB framebuffer 쓰기와 기존 사각형 그리기 경로.

원시 메모리 접근과 페이지 테이블 조작은 HAL, 물리 프레임의 소유권은 core, Limine 해석과 결합은 image에 둔다. 일반 드라이버·파일시스템·네트워크는 아직 구현하지 않았다.

## 검증

[검증 요약](validation/2026-10-31-memory/evidence.json)에 환경과 결과를 기록했다.

| 검증 | 결과 |
| --- | --- |
| no_std workspace check | 통과 |
| 전체 Rust 형식 검사 | 통과 |
| 모든 feature를 포함한 Clippy, 경고 오류 처리 | 통과 |
| 호스트 시험 | 33개 통과: 구간 5, 정규화 11, 프레임 풀 12, HAL 주소·테이블 검증 5 |
| 정규화 모델 | 2,000개 생성 지도를 독립 페이지별 판정과 비교 |
| 프레임 모델 | 20,000회 연산, 소진·반환·예약 제외·실패 후 상태·호스트 스레드 배타성 |
| 소유권 컴파일 검사 | 복제, private 필드 생성, 이중 반환, 풀보다 긴 수명 모두 거부 |
| debug/release ISO QEMU TCG | 각 1회, 프레임·RAM 초기화·128회 매핑/해제·ABC 통과 |
| 일반 debug 부팅 | boot-tests 없이 프레임 풀 활성화·ABC 통과; 반복 검사는 실행하지 않음 |

QEMU 검사는 3개의 테이블 프레임과 2개의 데이터 프레임을 유지하며 두 데이터를 번갈아 매핑한다. 페이지 전체 512개 u64를 가상 주소로 쓰고 HHDM 별칭으로 대조한다. 매핑 중복·빈 슬롯 해제·점유한 PML4 생성 실패도 확인한다. 8회 branch 생성/분리와 총 128회 매핑/해제 후 테이블 프레임을 포함한 여유 수가 복구된다. 초기화 검사는 페이지 전체를 nonzero로 채운 뒤 전체가 zero인지 확인한다.

debug 로그는 [여기](validation/2026-10-31-memory/debug.log), release 로그는 [여기](validation/2026-10-31-memory/release.log), 일반 부팅 로그는 [여기](validation/2026-10-31-memory/normal-debug.log)에 보존했다. 기존 build/debug.log는 덮어쓰지 않았다.

마지막 ABC 관측 후 호스트가 자신이 시작한 QEMU만 종료했다. guest의 종료나 실기 동작·성능을 검증한 것은 아니다. 구조 이동 당시의 별도 증거는 [이전 기록](validation/2026-10-30-layout-notes.md)에 남아 있다.

## 현재 한계와 다음 관문

- 부팅 매핑은 단일 CPU, maskable IRQ 비활성화, 같은 CR3, 배타적인 테이블 수정이 전제다. PCID·LA57는 지원하지 않는다.
- 프레임 풀 메타데이터는 정적이다. 현재 구성은 합계 4GiB 후보 RAM과 288개 결과 구간까지다. 물리 주소 상한이 4GiB라는 뜻은 아니며 용량 초과는 오류다. 동적 메타데이터 공급·큰 연속 할당·CPU별 캐시는 후속이다.
- 새 leaf만 NX다. Limine의 기존 HHDM 권한은 유지한다. 기존 HHDM의 RWX 별칭이 있으므로 전체 backing frame W^X를 달성했다고 주장하지 않는다.
- BootPageSlot은 전용 PML4 branch의 단일 부팅용 매핑이며 일반 사용자 AddressSpace가 아니다.
- BootloaderReclaimable·ACPI·부팅 페이지 테이블은 계속 보존한다.
- kernel heap, 일반 주소 공간, 사용자 진입·syscall·스케줄러·capability·IPC·DMA는 미구현이다. ICE·BetAFS의 실제 Kryo 실행·I/O 통합도 미구현이다.

다음은 주소 공간과 매핑 객체가 프레임 참조를 유지하도록 만들고, 사용자·커널 매핑 분리와 HHDM 별칭 권한을 포함한 실행 메모리 정책을 구현하는 관문이다. 이후 Ring 3 진입과 사용자 오류 격리로 이어간다.

## 작업 방식

기준은 [개발 규칙](development.md)과 [핵심 설계 계약](design-contract.md)이다. 구현과 시험·설계 검토를 완료 기준 하나씩 묶어 진행한다.
