tm-menu-controls = i TM 가져오기 · e TM 내보내기 · t 가르치기
    방향키 선택 · Enter 사용 · d 상세 · Esc 닫기
tm-import-path = TM 가져오기: 파일 경로 직접 입력
tm-export-path = TM 내보내기: 새 파일 경로 (덮어쓰기 금지)
tm-path-controls = 경로 입력 · Enter 계속
    Esc 취소 · 변경 없음
tm-path-boundary = 링크·비밀 경로·상위 이동·확장 금지. 64 KiB 이하 TOML만 허용.
tm-import-title = TM 가져오기 · 배울 로컬 파트너 선택
tm-import-summary = { $name } / { $kind } / { $form }
tm-import-partner = 배울 파트너: { $partner }
tm-import-controls = 위/아래 파트너 · Enter 이름/범위
    PgUp/PgDn 단계 · Esc 취소
tm-import-boundary = 명시적 로컬 학습만 기록. 이전 교사·결과·권한 없음. 없는 핀은 사용 시 일반 캡처 검토 전까지 사용 불가.
tm-import-action = TM 가져오기
tm-export-title = 새 TM 파일만 만들기 · 실행 없음
tm-export-details = 출력: { $path }
    { $definition }
    동작 정의만 포함. 로컬 ID·교사·증거·세션·승인 제외.
tm-export-controls = [유지] / 새 파일 만들기 · Enter 유지 · y 만들기
    좌/우 선택 · PgUp/PgDn 상세 · Esc 취소
tm-export-armed = 유지 / [새 파일 만들기] · Enter 만들기
    좌/우 선택 · PgUp/PgDn 상세 · Esc 취소
tm-path-refused = 경로 입력 거부: 제어 문자 없는 한 줄, UTF-8 4096바이트 이하.
tm-export-saved = 새 TM 파일로 내보냈습니다. 승인이나 증거는 포함하지 않습니다.
cmd-move-import = 휴대용 TM 파일 가져오기 · 로컬 학습 검토
cmd-move-export = 새 TM 파일로 기술 내보내기 검토
tm-free-slot = 빈 슬롯 { $slot }/4 · 명시적 로컬 학습
tm-semantic-old = 기존 정의 / 기술 #{ $id }, 리비전 { $revision }
tm-semantic-new = 제안 / 정확한 로컬 정의
tm-semantic-changed = 동작 정의가 바뀝니다. 과거 사용 기록은 원래 기술에 남고, 가르친 사본은 독립적으로 유지됩니다.
tm-semantic-equal = 동작 정의가 같습니다 (argv 순서, 정규화된 핀 집합 기준). 새 실행 증거는 아닙니다.
tm-semantic-name-only = 이름만 수정합니다. 단계·범위·출처·과거 사용 기록은 그대로 유지됩니다.
tm-semantic-boundary = 이 로컬 목록만 변경합니다. 실행·요청·핀 선택·권한 부여 없음. 취소하면 목록과 원래 초안·커서가 유지됩니다.
tm-semantic-scope = 로컬 수신자: { $partner } / 파트너 #{ $id } / { $form }
    범위: { $scope } / 로컬 이름: { $name }
tm-semantic-argument = argv[{ $index }] (0부터): { $old } -> { $new }
tm-semantic-cwd = cwd: { $old } -> { $new }
tm-semantic-kind = 동작 종류: { $old } -> { $new }
tm-semantic-pins = 핀 선택자가 바뀝니다. 아래에서 기존·제안 경로와 범위를 모두 확인하세요.
tm-semantic-instruction = 사용자 지침이 바뀝니다. 아래에서 기존·제안 지침을 모두 확인하세요.
tm-semantic-goal = 목표가 바뀝니다. 아래에서 기존·제안 목표를 모두 확인하세요.
tm-semantic-criterion = 기준[{ $index }] (0부터): { $old } -> { $new }
tm-semantic-absent = <없음>
tm-semantic-stale = 검토가 오래되었거나 사용할 수 없습니다. 취소 후 새로 검토하세요. 변경 없음.
tm-semantic-expose = 로컬 변경 확인: PgDn으로 기존·제안 상세를 모두 확인하세요.
    Enter 유지 · Esc 취소. 숨겨진 범위는 승인할 수 없습니다.
tm-semantic-slot = 로컬 슬롯 { $slot }/4 교체 / 기술 #{ $id } 리비전 { $revision }
tm-semantic-teacher = 로컬 교습 원본: 기술 #{ $id }, 리비전 { $revision }. 원본은 그대로 유지되며 새 사본은 독립적입니다.
tm-semantic-fresh = 새 로컬 정의입니다. 기존 기술은 변경되지 않으며 과거 실행 증거를 이어받지 않습니다.
tm-semantic-literals = 따옴표 안은 셸 문법이 아닌 리터럴입니다. ""는 빈 값, <없음>은 항목 없음이며 이스케이프는 정확한 공백을 보존합니다.
tm-semantic-reminder = 원래 기술 #{ $id }, 리비전 { $revision }을 동일한 단계와 기존 사용 기록 그대로 다시 배웁니다.
