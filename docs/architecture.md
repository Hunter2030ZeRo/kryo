# Kryo 아키텍처 기준

이 문서는 현재 소스와 관련 설계 대화를 대조해 정리했다. 세부 수명·권한 규칙은 [핵심 설계 계약](design-contract.md)을 따른다.
구현 상태는 [status.md](status.md), 각 디렉터리의 책임은 [layout.md](layout.md)에 구분한다.

## 확정된 방향

1. Cryolite의 커널은 Kryo다. Rust `no_std`, 우선 x86-64, Limine 부팅을 사용한다.
2. Ring 0은 실행·메모리·권한·통신을 위한 기구를 제공한다. 일반 장치 드라이버, VFS, 파일시스템, 네트워크 스택은 Ring 3의 독립 서비스다. 이전의 Ring 0 드라이버/FS 하이브리드 제안은 폐기된 방향이다.
3. 성능·자원 효율과 고장 격리를 함께 목표로 한다. 서비스 재시작·교체 시 재부팅을 피하도록 설계하되, 장치 및 진행 중인 요청의 복구가 선행되어야 한다.
4. 공통 HAL 인터페이스는 정적 선택·제네릭 단형화를 우선 검토한다. 하드웨어별 기능을 core에서 직접 가져다 쓰지 않는다. 현재 HAL은 예외 초기화, 진단 출력, framebuffer와 BSP 부팅 매핑 기구를 제공한다.
5. 공유 버퍼를 중요한 데이터 경로로 삼는다. ABI와 소유권·권한·수명 계약을 먼저 정하고, 복사량·문맥 전환·깨우기 비용을 측정한다.
6. Unix-like/POSIX 활용을 염두에 둔다. 네이티브 커널 ABI를 Linux나 NT의 syscall 테이블과 동일하게 만들지는 않는다.
7. ICE-Linux와 ICE-NT는 공통 네이티브 기능에 각각 연결한다. NT를 Linux 호환성 위에 쌓지 않는다. macOS/XNU는 후순위다.
8. 파일시스템은 후속 대화에서 BetAFS로 이름이 정해졌으며 Kryo와 독립적으로 개발한다. `userspace/filesystems/betafs/`는 Kryo 연결부의 자리다. 과거 CryoFS/LSM 참고 논의를 이 저장소의 확정 디스크 형식으로 취급하지 않는다.
9. 사용자의 승인에 따라 구현·검증은 자동 진행하며 사용자가 필요한 코드를 검토·수정한다. 읽기 쉬운 이름과 함수, 필요한 안전 조건·설계 이유만 담은 주석을 사용한다. 실제 기능과 검증을 한 묶음씩 추가한다.

## 계층과 의존성

```text
kernel/image ──→ kernel/core ──→ hal
       │               └─────→ interfaces
       └─────────────────────→ hal   (현재 초기화 연결)

userspace 서비스 / ICE ──→ 사용자 런타임·libsys ──→ interfaces
                      └──── IPC·공유 메모리 ────→ 다른 Ring 3 서비스
```

화살표는 Rust 의존성 또는 명시된 사용자 통신 경계다. Ring 3 프로세스가 `kryo-core`나 `kryo-hal`을 링크해 특권 코드를 호출하는 구조가 아니다.
현재 `kryo-core`는 메모리 정규화·프레임 소유권을 구현하며, `kryo-interfaces`의 사용자 계약은 아직 미구현이다. 그림의 사용자 영역은 계획이며 Cargo에 등록하지 않았다.

- `hal`은 CPU 진입, 레지스터, IRQ 컨트롤러, 페이지 테이블, TLB, IOMMU 같은 하드웨어 조작을 맡는다.
- `kernel/core`는 프레임·주소 공간·태스크·권한·IPC·스케줄링의 수명과 불변 조건을 맡는다.
- `interfaces`에는 버전이 있는 계약과 표현만 둔다. 서비스 구현, 하드웨어 접근, 할당 정책을 넣지 않는다.
- `kernel/image`는 부트로더 응답 해석, 구성요소 결합, `_start` 및 panic 경로를 맡는다.
- 장치 발견/드라이버 선택은 `device-manager`, 파일 namespace는 `vfs`, 파일 페이지 공급은 `pager`, 서비스 복구 정책은 `supervisor`에 둔다.
- 인터럽트 컨트롤러·MMU 같은 필수 CPU 기구와 초기 debugcon은 Ring 0에 남는다. 이것이 일반 장치 드라이버를 Ring 0에 넣는 근거는 아니다.

`unsafe`는 작은 경계에 모으고 각각의 전제와 소유권을 설명한다. HAL 외에도 사용자 포인터 검증, 메모리 할당기 등 core의 경계에는 검토할 `unsafe`가 있을 수 있다.

## 부트스트랩과 고장 격리

초기 사용자 프로세스와 서비스 manifest는 `boot/initramfs/`에 준비할 입력이다. 커널의 초기 로딩은 아직 존재하지 않는 VFS나 저장장치 서비스에 의존하면 안 된다.
Limine에서 전달받은 초기 이미지로 init을 시작하고, 최소 권한을 배분한 뒤 감독자·장치 관리자·파일시스템을 올리는 순서로 설계한다. 구체적인 이미지 포맷은 미정이다.

서비스가 죽어도 필요한 참조가 남은 공유 메모리 객체는 독립적으로 유지할 수 있게 한다. 새 서비스가 이전 완료를 받아들이지 않도록 세대와 요청 식별자를 검증한다.
드라이버 재시작만으로 DMA가 멈추지는 않는다. 정지/배출/필요한 장치 reset과 DMA 권한 철회를 포함하는 복구 프로토콜이 필요하다.

## 아직 고정하지 않은 선택

- HAL trait의 구체적 메서드, 네이티브 syscall 번호와 레지스터 규약, IPC wire layout.
- 사용자 타깃·libc·executor. 현재의 `x86_64-unknown-none` 설정은 커널용이며 사용자 공간 전체의 빌드 환경이 아니다.
- 연속 프레임·kernel heap 할당 정책. 현재 단일 프레임은 비트맵으로 관리한다. 미사용 후보 allocator 의존성은 두지 않고 실제 heap 구현 시 하나를 선택한다.
- FRED 사용, 스케줄러 정책, 페이지 크기 정책, SMP 상세 구조.
- 이전 제안에 등장한 x86-64-v3 최소 기준의 실제 적용. 현재 rustflags에는 `target-cpu=x86-64-v3`가 없으며 이번에는 CPU 명령어 기준을 바꾸지 않았다.
- 커널 자체의 부분 갱신은 연구 항목이다. Ring 3 서비스 교체와 동일한 수준으로 보장하지 않는다.

## 근거가 된 대화

사용자의 수정·선택을 이전 답변의 제안보다 우선했다. 다음은 기록의 출처이지 외부 기술 표준이 아니다.

- [Kryo 커널 구조 설계](chatgpt-conversation://6ae3ebc6-26cc-83ec-8d29-3b9ade4aac80): Ring 3 드라이버/FS 정정, 공유 버퍼, 고성능·복구 목표.
- [Kryo 개발 우선순위](chatgpt-conversation://6ae3f77f-7a1c-83ec-b859-d871637bb501): 정적 HAL, 독립 프로젝트의 역할, 재사용 방향.
- [ICE와 Kryo 병행 개발](chatgpt-conversation://6ae3e7f0-817c-83ec-9dfd-d5bfdacd3b27): Linux·NT 독립 연결과 macOS 후순위.
- [MORAX 기반 운영체제 비교](chatgpt-conversation://6ae18bf8-8f2c-83ec-9934-37e2b10f8175): Unix/POSIX 지향 및 초기 FS 방향.
- [Cryolite 파일시스템 설계](chatgpt-conversation://6ae3f4b2-11a4-83ec-96bd-a8d9862ce5a2): BetAFS 명명과 독립 개발.

## 빌드 구성 근거

Cargo workspace는 lockfile·target·profile을 루트에서 관리한다. 커널 링커 옵션은 image의 `build.rs`에서 해당 바이너리에만 전달한다.
[Cargo workspace 문서](https://doc.rust-lang.org/cargo/reference/workspaces.html), [바이너리별 링커 옵션](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rustc-link-arg-bin)을 기준으로 구성했다.
