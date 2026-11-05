# 원본 전환 점검

이 문서는 Python `log-generator`를 Rust로 전환할 때 확인할 범위와 남은 차이를 기록합니다. 동작이 있다는 사실만으로 원본과 같은 결과를 보장하지 않습니다.

## 확인한 범위

| 영역 | 현재 구현과 확인 근거 |
| --- | --- |
| CLI | 원본 명령의 이름과 주요 옵션을 제공합니다. `tests/cli.rs`에서 설정, 로그 생성과 제어 명령을 검사합니다. |
| 로그 유형 | 원본의 생성기 유형과 Redis, SSH 인증, Kafka broker, HAProxy HTTP 유형을 제공합니다. `tests/builtins.rs`, `tests/original_formats.rs`와 `scripts/check_original_formats.py`로 형식을 확인합니다. |
| 출력 | 파일, 여러 파일, 콘솔, JSON, TCP, UDP, HTTP 출력을 제공합니다. `tests/output.rs`와 `tests/cli.rs`에서 해당 경로를 검사합니다. |
| 패턴과 시나리오 | 패턴 관리, 필드 생성, 빈도와 오류 시나리오를 제공합니다. `tests/pattern_manager.rs`, `tests/frequency.rs`, `tests/scenario.rs`에서 검사합니다. |
| 라이브러리 | 생성기 등록, 엔진 제어, 성능 측정과 출력 API를 제공합니다. `tests/registry.rs`, `tests/engine.rs`, `tests/performance.rs`에서 검사합니다. |

2026-11-05에 Linux arm64 컨테이너에서 전체 테스트와 배포 이미지의 설정 검증 및 로그 생성을 실행했습니다. 이 테스트 환경에는 Python이 없어 Python 어댑터 테스트를 건너뛰었습니다. 배포 이미지에서는 Redis, SSH 인증, Kafka broker 로그 생성을 확인했습니다.

같은 날 최신 코드에 Python을 설치한 Linux arm64 컨테이너에서 전체 테스트를 다시 실행했습니다. Python 어댑터 테스트를 포함해 통과했습니다.

이후 생성기와 분석을 수정한 코드는 Python이 없는 Linux arm64 컨테이너에서 전체 테스트를 다시 통과했습니다. Python 어댑터 실행은 이 재검사에 포함되지 않았습니다.

구조화 `LogEntry` 출력 API를 추가한 뒤 Linux arm64에서 출력과 엔진 테스트 44개를 다시 실행해 통과했습니다. Windows 대상 전체 컴파일 검사도 통과했습니다.

구조화 생성기를 라이브러리와 CLI 엔진에 연결한 뒤 Linux arm64에서 관련 테스트 51개를 통과했습니다. Windows 대상 전체 컴파일 검사도 다시 통과했습니다.

같은 코드로 Python을 설치한 Linux arm64 컨테이너에서 프로세스 플러그인 테스트 17개를 별도로 실행해 통과했습니다.

최신 Linux 이미지에서 Redis, SSH 인증, Kafka broker 로그를 각각 100건 생성하고 같은 이미지의 검증기로 검사했습니다. 세 유형 모두 통과했고 같은 seed와 시작 시각으로 실행한 결과가 일치했습니다.

macOS의 release 빌드에서 Redis, SSH 인증, Kafka broker 로그를 각각 10,000건 생성했습니다. 검증과 분석 명령은 세 유형의 모든 레코드를 유효한 것으로 처리했습니다.

같은 이미지의 `log-generator` 명령으로 원본 설정 예제 아홉 개를 검증했습니다. 일부 설정만 포함한 파일에서도 원본 기본 설정 병합과 syslog 생성 4건을 확인했습니다.

아홉 설정 예제에서 기본 로그 다섯 종류를 각각 10건씩 생성해 같은 이미지에서 검증했습니다. 콘솔 출력과 가상 시간을 지정했고, 모니터 포트를 자동 할당하며 분산 추적을 껐습니다. 설정에 지정된 출력 대상과 추적 기능은 이 검사에 포함하지 않습니다.

다중 파일 출력 예제 다섯 개도 Linux 이미지에서 실행했습니다. 예제마다 Nginx와 syslog 로그 총 100건을 생성하고 파일명, 날짜와 형식을 검사했습니다. 출력 경로는 임시 디렉터리로 지정했습니다.

Linux 이미지에서 `multi_file_config.yaml`의 1MB 회전 설정으로 Nginx 로그 8,000건을 생성했습니다. 활성 파일과 회전 파일 두 개의 레코드 합계와 형식을 검사했습니다.

Linux arm64 컨테이너에서 고급 예제의 파일·콘솔·TCP 출력과 마이크로서비스 예제의 파일·TCP·HTTP 출력을 로컬 수신기로 검사했습니다. 이 검사는 외부 서비스 대신 임시 파일과 로컬 포트를 사용합니다.

Linux arm64에서 서비스 간 trace 연결, metrics·health 엔드포인트와 gateway 지표 테스트도 통과했습니다. 이 테스트는 로컬 생성 데이터와 수신기를 사용합니다.

Linux arm64에서 Unix 제어 소켓의 상태 조회·일시 정지·재개·종료, 모니터 출력, 종료 신호, TCP 타임아웃과 재연결 테스트가 통과했습니다.

같은 날 Linux x86_64 에뮬레이션 컨테이너에서 Python을 설치한 뒤 전체 테스트를 순차 실행했습니다. Python 어댑터 테스트를 포함해 모두 통과했습니다.

오류 지속과 시간 모드의 기본 동작을 변경한 뒤 Linux arm64 컨테이너에서 CLI, 설정과 registry 테스트 218개가 통과했습니다. 이 검사는 Windows 실행과 Python 어댑터 실행을 포함하지 않습니다.

최신 코드의 내장 생성기와 원본 형식 테스트 14개도 Linux x86_64 에뮬레이션 컨테이너에서 통과했습니다. 내장 생성기 테스트는 원본 17종과 Redis, SSH 인증, Kafka broker를 포함합니다.

최신 Docker 이미지를 다시 빌드한 뒤 이미지의 `log-generator` 명령으로 기본 설정을 검증했습니다. Redis, SSH 인증, Kafka broker 로그를 각각 100건 생성했습니다. 이 검사는 생성 건수를 확인합니다.

## 남은 확인과 차이

- `log-generator`는 설정 파일에 원본 기본 설정을 자동 병합합니다. `log-forge`는 `--original-defaults`를 지정할 때 병합합니다.
- Windows 제어 명령은 로컬 named pipe를 사용합니다. Windows 실제 실행 검증은 이번 작업 범위에서 제외했습니다.
- Python 확장은 별도 Python 환경과 어댑터가 필요합니다. Rust 확장은 다른 인터페이스를 사용합니다.
- 원본은 회전 파일에 시간 문자열을 쓰고 한도를 넘으면 오래된 파일을 삭제합니다. Rust는 숫자 확장자를 쓰고 기존 백업을 보존하며 한도에 도달하면 중지합니다.
- 원본 엔진과 Rust `log-generator` 명령은 생성 또는 출력 오류를 기록하고 생성을 계속합니다. `--fail-fast`를 지정하면 중지합니다. Rust 라이브러리와 `log-forge` 명령은 기본적으로 오류를 반환하고 중지합니다. 라이브러리에서는 `set_continue_on_error(true)`, CLI에서는 `--continue-on-error`로 레코드 오류 후 계속할 수 있습니다. CLI는 오류 지속 모드에서 각 출력 대상을 시도하고 생성 성공을 건수에 포함하며, 배치 flush 오류 후에도 계속합니다. 종료 시 close 오류는 stderr에 기록하고 성공 상태를 반환합니다. 비동기 출력에서는 각 출력 결과를 기다립니다. 병렬 생성에서 실패한 레코드는 가상 타임스탬프를 소비합니다. 지속적인 생성 오류는 사용자가 중지할 때까지 반복됩니다. 라이브러리는 실패한 생성에도 입력 타임스탬프를 소비하며, flush와 close 오류를 반환합니다.
- 원본 `LogEntry`는 선택 필드의 빈 문자열과 0을 JSON에서 생략합니다. 기본 문자열에서도 빈 `source_ip`와 0인 `status_code`, `response_time`을 생략합니다. Rust는 이 값을 보존합니다. 원본 `GenerationStatistics`는 시간 차이가 0 이하이면 이전 생성 속도를 유지하지만 Rust는 0을 반환합니다.
- 로그 분포와 일부 사용자 설정은 원본과 다릅니다. 각 생성기와 설정을 입력별로 대조해야 합니다.
- 시나리오의 추가 필드는 보존하지만 생성에 적용하지 않습니다. 원본도 `start_pattern`과 `escalation_probability`를 패턴 선택에 사용하지 않습니다. 시나리오 관리자에 빈도를 설정하면 내보내기에 포함되며, `generate_if_due`가 빈도를 적용합니다.
- 시나리오 우선 생성은 활성 시나리오가 없으면 일반 오류를 선택할 수 있습니다. 두 경로의 사용 이력은 관리자 상태와 오류 라이브러리에 반영됩니다.
- 필드 빈도의 웹·보안·데이터베이스 프리셋을 제공합니다. Rust 데이터베이스 프리셋은 행 수가 1,000보다 크면 설정된 확률로 실행 시간을 선택합니다. 원본은 조건 함수를 값과 직접 비교해 이 상관관계를 실행하지 않습니다.
- 원본 `LogFactory`는 Docker의 최상위 `format_type`을 생성자에 전달하지 않습니다. Rust 설정은 이 값을 적용합니다.
- Docker JSON의 `log` 값을 디코딩하면 원본은 끝에 문자 `\n`을 두고, Rust는 실제 줄바꿈 문자를 둡니다. Rust 동작은 Docker 로그의 줄바꿈을 표현합니다.
- 원본의 아홉 설정 예제가 파싱되는 것만으로 모든 설정 조합과 출력 의미가 검증되지는 않습니다.
- 원본과 Rust의 속도 차이는 동일한 데이터, 출력 대상과 실행 환경에서 측정해야 합니다. 저장소의 벤치마크 결과를 다른 환경에 일반화할 수 없습니다.

2026-11-05에 macOS에서 `cargo check --locked --all-targets --target x86_64-pc-windows-gnu`가 통과했습니다. 이 검사는 Windows에서의 링크와 실행을 확인하지 않습니다.

같은 날 macOS에서 Cargo 패키지를 임시 경로에 설치했습니다. 설치한 `log-forge`의 버전 출력과 `log-generator`의 설정 검증 및 Redis 로그 생성을 확인했습니다.

Docker와 Kubernetes 기본 identity 변경 후 Cargo 패키지 빌드를 다시 검증했습니다. 두 유형의 원본과 Rust 표본을 각각 100건씩 교차 검증했을 때 거부된 레코드는 없었습니다.

## 처리량과 메모리 비교

2026-11-05에 macOS arm64에서 Nginx 접근 로그 10,000건을 콘솔로 출력하는 실행을 각각 세 번 측정했습니다. 두 프로그램의 생성 결과와 난수 분포는 동일하지 않습니다. 시간에는 프로세스 시작과 출력이 포함됩니다.

```sh
cargo build --release --locked
python3 benchmarks/compare_cli.py --original /tmp/log-forge-original --count 10000 --runs 3
```

Python의 실행 시간 중앙값은 4.181초, Rust는 0.0177초였습니다. Rust의 첫 실행은 0.336초였습니다. 최대 상주 메모리 중앙값은 Python 44,351,488바이트, Rust 9,191,424바이트였습니다. 이 결과는 해당 장비와 설정에서 측정한 값입니다.

오류 지속을 기본으로 적용한 뒤 같은 장비에서 Rust `log-generator` 명령으로 세 번 다시 측정했습니다. 실행 시간 중앙값은 Python 4.193초, Rust 0.0178초였습니다. 최대 상주 메모리 중앙값은 Python 44,400,640바이트, Rust 9,306,112바이트였습니다. 두 프로그램이 생성한 내용과 난수 분포는 같지 않습니다.

```sh
python3 benchmarks/compare_cli.py --original /path/to/log-generator --binary target/release/log-generator --count 10000 --runs 3
```

## 원본 형식 비교

내장 오류 패턴 15개와 원본 기본 시나리오 4개의 설정은 원본과 일치합니다. 다음 명령은 오류 패턴의 생성 시각을 제외하고 모든 필드를 비교합니다. Rust 데이터에서 생략한 예제 목록은 빈 목록으로 처리합니다. Rust에 추가한 시나리오는 별도로 허용합니다.

```sh
python3 scripts/check_original_error_patterns.py --original /path/to/log-generator
```

원본 생성기의 메시지 패턴 13개 그룹과 syslog·Goloop 패턴 데이터도 비교할 수 있습니다.

기본 설정을 제외한 생성 실행에서 Nginx·Apache·Django의 기본 URL, FastAPI endpoint와 Nginx·Apache user agent를 확인했습니다. 각 유형에서 20,000건을 생성했을 때 확인 대상 84개가 모두 출력됐습니다. 또한 Nginx, Apache, MySQL, Docker, Kubernetes, FastAPI에서 유형별 30,000건을 생성해 메시지 패턴 13개 그룹의 각 항목이 출력되는 것을 확인했습니다. 이 표본 검사는 설정별 분포를 검증하지 않습니다.

Goloop의 모듈 전체를 활성화해 50,000건을 생성했을 때 기본 메시지 패턴 97개가 모두 출력됐습니다.

Syslog의 12개 facility에서 각각 3,000건을 생성해 기존 메시지 패턴 80개가 모두 출력되는 것을 확인했습니다.

PostgreSQL 로그 30,000건에서는 기존 SQL 유형과 오류 메시지를 대표하는 패턴 28개가 모두 출력됐습니다.

```sh
python3 scripts/check_original_generator_patterns.py --original /path/to/log-generator
```

2026-11-05에 macOS에서 원본 생성기 17종을 각각 100건씩 비교했습니다. 실행에는 원본 저장소와 Python 의존성이 필요합니다.

```sh
python3 scripts/check_original_formats.py --original /tmp/log-forge-original --samples 100
```

Rust 검증기는 원본 출력 1,700건과 Rust 출력 1,700건을 모두 허용했습니다. 원본 검증기는 15종의 Rust 출력 1,500건을 모두 허용했습니다. 나머지 두 유형에서는 Django traceback 8건과 Goloop 출력 31건을 거부했습니다. 원본 생성기의 출력에서도 각각 11건과 23건을 거부했습니다.

이 비교는 출력 형식만 검사합니다. 분포, 모든 설정 조합과 실제 운영체제의 동작은 확인하지 않습니다. 원본 생성은 무작위이므로 거부 건수는 실행마다 달라질 수 있습니다.

원본 Python 생성기를 어댑터에서 실행하는 경로는 Nginx common, Docker text와 syslog RFC 5424 형식을 각각 10건씩 검사했습니다. Docker 원본 생성기는 요청한 가상 시간을 적용하지 않습니다.

```sh
python3 scripts/check_original_adapter.py --original /tmp/log-forge-original
```

아홉 설정 예제에서 기본 로그 유형 다섯 개를 각각 선택해 10건씩 생성하는 테스트도 있습니다. 테스트는 콘솔 출력과 가상 시간을 지정하고, 서비스 로그가 필요한 분산 추적을 끕니다. 이 테스트는 생성 형식을 확인하며 설정된 출력 대상의 동작을 대신 검증하지 않습니다.

다중 파일 출력 예제 다섯 개는 Nginx와 syslog 레코드 100건씩 생성해 파일 이름의 유형과 날짜, 레코드 형식을 검사합니다. 이 테스트는 회전 한도에 도달하지 않습니다.

`multi_file_config.yaml`의 1MB 회전 설정은 Nginx 로그 8,000건으로 검사합니다. 활성 파일과 회전 파일에서 읽은 레코드 합계가 생성 건수와 일치합니다.

## 접근 로그 분포 비교

다음 명령은 원본과 Rust에 같은 사용자 설정을 적용해 유형별 100건을 검사합니다. Nginx와 Apache의 상태 코드·user agent·오류 수준, MySQL의 질의 종류·테이블·오류 수준, Django의 요청 수준·SQL 모델을 확인합니다. PostgreSQL·FastAPI의 수준, Kubernetes의 수준과 이벤트 유형, syslog의 facility·severity·hostname, Goloop의 수준과 모듈도 확인합니다.

```sh
python3 scripts/check_configured_generators.py --original /tmp/log-forge-original --count 100
```

2026-11-05에 두 구현의 2,800건이 선택한 설정을 모두 반영했습니다. MySQL 관리 명령에는 질의 종류 설정이 적용되지 않습니다. Rust의 `Field List` 명령은 테이블 설정을 사용합니다. 이 검사는 다른 설정이나 분포의 동등성을 확인하지 않습니다.

같은 날 MySQL 일반 로그를 각 10,000건 생성했습니다. SELECT 중 기본·WHERE·JOIN 비율은 원본이 각각 29.2%·42.5%·28.3%, Rust가 29.1%·42.1%·28.8%였습니다. 이 수치는 단일 표본입니다.

PostgreSQL SELECT의 기본·WHERE·JOIN 비율은 원본 표본 10,000건에서 29.9%·42.1%·28.0%였습니다. Rust 표본 14,891건에서는 29.6%·42.3%·28.0%였습니다. Rust 표본은 LOG 수준과 SELECT 질의를 지정한 로그에서 추출했습니다.

Rust PostgreSQL 로그 10,000건은 검증과 분석을 모두 통과했습니다. 분석의 수준별 합계도 생성 건수와 일치했습니다.

Django 요청 로그를 INFO 수준으로 각각 10,000건 생성한 표본에서 GET 비율은 원본 65.2%, Rust 65.0%였습니다. 익명 사용자 비율은 각각 69.7%와 70.4%였습니다. 1,000ms 초과 요청은 각각 1.0%와 0.8%였습니다.

FastAPI 로그는 서로 다른 seed 두 개로 각각 10,000건씩 파일에 생성했습니다. Rust 검증과 분석 명령은 모든 레코드를 유효한 것으로 처리했습니다. 원본과 Rust의 FastAPI 표본을 각각 100건씩 교차 검증했을 때 거부된 레코드는 없었습니다.

Kubernetes event 10,000건도 Rust 검증과 분석 명령을 통과했습니다. 분석 결과는 `INFO` 6,953건과 `WARNING` 3,047건이었습니다. 원본과 Rust의 event 표본을 각각 100건씩 교차 검증했을 때 거부된 레코드는 없었습니다.

syslog의 원본 메시지 시설 12개에서 각각 1,000건을 생성해 Rust 검증기로 검사했습니다. 12,000건 모두 통과했습니다. 원본과 Rust 표본도 각각 100건씩 교차 검증했습니다.

같은 실행에서 Docker JSON을 각각 100건 검사했습니다. 원본 100건은 `log` 값이 문자 `\n`으로 끝났고, Rust 100건은 실제 줄바꿈 문자로 끝났습니다.

다음 명령은 원본과 Rust의 기본 Nginx·Apache 접근 로그를 각각 생성하고 HTTP method와 status 비율을 출력합니다. 설정 파일로 바꾼 분포는 비교하지 않습니다.

```sh
python3 scripts/compare_distributions.py --original /tmp/log-forge-original --count 5000
```

2026-11-04에 유형별 5,000건을 비교했을 때 항목별 최대 비율 차이는 Nginx method 1.06%p, status 1.30%p였습니다. Apache는 각각 2.38%p와 1.32%p였습니다. 이 수치는 표본 결과이며 분포가 같다는 증거는 아닙니다.

## 완료 기준

1. 원본에서 사용한 로그 유형과 패턴을 Rust에서 생성합니다.
2. 추가 로그 유형을 생성하고 출력 형식을 검사합니다.
3. 원본 예제와 대표 설정을 실행하고, 오류와 종료 동작을 검사합니다.
4. macOS와 Linux에서 실행과 제어 명령을 검사합니다.
5. 같은 조건에서 처리량과 메모리를 측정하고 재현 명령을 기록합니다.

Python 구현의 모든 동작이나 API를 동일하게 재현할 필요는 없습니다.
