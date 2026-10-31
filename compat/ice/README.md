# ICE

`linux/`와 `nt/`는 native syscall·IPC·사용자 서비스에 독립 연결한다. 한 호환성 엔진이 다른 엔진의 번역 결과에 의존하지 않는다.
`userspace/`는 호환 로더·라이브러리 연결 자리다. macOS/XNU는 후순위다.
현재 구현과 실행 파일은 없다. 커널 코어에 Linux/NT 전용 타입이나 syscall dispatch를 넣지 않는다.
