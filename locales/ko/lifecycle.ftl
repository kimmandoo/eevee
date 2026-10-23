lifecycle-command-update = 체크섬 검증 앱 업데이트 검토 · 아직 교체하지 않음
lifecycle-command-rollback = 보관 패키지 롤백 검토 · 데이터 호환성 입증 필요
lifecycle-update-title = 앱 업데이트 · 교체 전 검토
lifecycle-rollback-title = 보관 패키지 롤백 · 호환성 확인 필요
lifecycle-waiting = 초안·로컬 저장 완료와 작업 대기 상태를 기다립니다. 설치는 시작하지 않았습니다. 진행 중인 작업을 마친 뒤 r로 다시 확인하거나 Esc로 돌아가세요.
lifecycle-expired = 탐색·작업·로컬 상태가 바뀌어 검토가 만료됐습니다. r로 새로 확인하세요. 승인된 변경은 없습니다.
lifecycle-unavailable = 업데이트/롤백을 사용할 수 없습니다. 이 확인 과정은 설치와 작업 데이터를 바꾸지 않았습니다.
lifecycle-failed = 설치 도우미가 실패했습니다. 터미널 출력과 복구 상세를 확인하세요. 교체나 복구가 성공했다고 가정하지 마세요. r로 현재 설치를 다시 확인하세요.
lifecycle-update-cache = 기존 스프라이트 캐시를 유지합니다. 없는 검증된 이미지는 설치기가 다운로드할 수 있으며, 설치가 실패해도 완료된 캐시 다운로드는 남을 수 있습니다.
lifecycle-rollback-cache = 보관된 오프라인 패키지만 사용합니다. 네트워크 요청과 스프라이트 캐시 변경은 없습니다.
lifecycle-update-data = 설치기는 작업·제공자 데이터를 교체하지 않습니다. 먼저 전체 데이터 폴더를 백업하세요. 새 버전이 다시 열릴 때 지원되는 데이터를 마이그레이션할 수 있습니다. 저장된 작업·초안을 이어가며 세션·승인은 복원하지 않습니다.
lifecycle-rollback-data = 정확한 영구 형식 식별과 현재 데이터 읽기 전용 검증이 필요합니다. SQL 스키마 번호가 같아도 호환성 증거가 아닙니다. 작업 데이터를 다시 쓰거나 백업을 복원하지 않습니다. 세션·승인 없이 저장된 작업·초안을 이어갑니다.
lifecycle-scope = 현재: { $current }
    대상: { $target }
    출처: { $source }
    설치 위치: { $directory }
    { "" }
    캐시: { $cache }
    { "" }
    데이터 / 이어가기: { $data }
    { "" }
    실행 가능 조건: { $reason }
    { "" }
    이 교체 한 번만 승인합니다. 기존 소유권·체크섬과 게시 실패 복구 검사를 유지합니다. 권한 상승이나 셸 프로필 수정은 없습니다.
lifecycle-choose-stay = [머무르기] / 한 번 적용 · Tab/Right 선택 · Enter 유지
    y 한 번 적용 · n/Esc 돌아가기 · r 새로 확인
lifecycle-choose-apply = 머무르기 / [한 번 적용] · Enter로 이 검토 적용
    y 한 번 적용 · n/Esc 돌아가기 · r 새로 확인
lifecycle-inspect = PgUp/PgDn으로 전체 범위를 읽은 뒤 승인하세요.
    Enter/n/Esc 설치 없이 돌아가기 · r 새로 확인
