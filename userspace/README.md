# 사용자 공간

모든 구성요소는 Ring 3의 별도 프로세스 또는 그 사용자측 라이브러리다. 장치 드라이버·파일시스템·VFS·네트워크 구현을 kernel/core에 넣지 않는다.
현재는 디렉터리 예약이며 Cargo 구성원이나 실행 가능한 프로그램이 아니다. 사용자 타깃·libc·런타임은 구현 시 결정한다.

`filesystems/betafs/`는 별도 BetAFS 프로젝트의 Kryo 어댑터 위치다. 저장 엔진을 여기에 복제하지 않는다.
전체 책임과 계약은 [디렉터리 지도](../docs/layout.md)와 [아키텍처](../docs/architecture.md)를 따른다.
