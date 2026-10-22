records-small = 기록 화면은 60열 × 18행 이상이 필요합니다. Esc로 돌아갑니다. 변경된 내용은 없습니다.
records-summary = 파트너 요약
records-routes = 폼 이동 기록 · 로컬 비증거 자료
records-dex = 실패 도감 · 현재 프로젝트
records-resolve = 과거 해결 관계 검토
records-habits = 반복해서 보낸 작성 지시
records-pairings = 파트너별 제공자 연결 설정
records-discard = 저장되지 않은 기록 버리기 검토
records-position = 항목 { $index } / { $count }
records-local-only = 로컬 메타데이터입니다. 제공자 요청 묶음이나 인계 문서에 포함되지 않습니다.
records-waiting = 저장 확정을 기다립니다. 대기 중인 기록의 원래 내용은 유지됩니다.
records-queue-full = 로컬 기록 한도에 도달했습니다. 저장을 마치거나 실패한 쓰기 작업을 검토한 뒤 계속하세요.
records-load-failed = 기록을 불러오지 못했습니다. /records-retry로 재시도할 수 있습니다. 읽기 실패는 종료를 막지 않습니다.
records-none = 없음
records-produced = 산출물 있음
records-stale = 오래된 상태
records-current = 현재 유효한 근거
records-unavailable = 사용할 수 없음
records-unknown = 알 수 없음 / 불명확
records-unknown-producer = 만든 파트너 알 수 없음
records-marker = { $status } · 산출물 { $count }개 · { $producer } · { $fact }
records-identity = { $name } · { $form } / { $element }
records-met = 만난 날: { $date } UTC { $origin } · 프로젝트: { $project }
records-derived = (기존 기록에서 추정한 날짜)
records-unknown-date = 날짜 알 수 없음
records-unknown-project = 프로젝트 알 수 없음
records-bond = 유대: 프로젝트·개인 공유 메모리 { $count }개 (이 파트너만의 개수가 아님)
records-no-moves = 아직 배운 기술이 없습니다.
records-form-counts = { $form }: 기술 { $moves }개 · 산출물이 있는 서로 다른 작업 { $tasks }개
records-unknown-artifacts = 만든 파트너를 알 수 없는 산출물 { $count }개는 이 파트너의 실적으로 세지 않습니다.
records-task-markers = 현재 작업 · 역할별 산출물 (생성 사실은 검증 결과가 아님):
records-empty = 아직 확정된 파트너 기록이 없습니다.
records-routes-disclosure = 폼 이동 기록은 로컬 탐색 일지이며 증거가 아닙니다. 흐리게 표시된 방문에는 해당 시점의 실제 파트너·폼이 만든 종료 산출물이 없습니다. 순서는 시각이 아닌 저장 순번을 따릅니다.
records-no-routes = 이 작업에서 이 파트너의 폼 전환 기록이 없습니다.
records-accepted-suggestion = { " " }[수락한 제안: { $trigger }]
records-route-receipt = 순번 { $ordinal } · { $date } UTC · { $reference }
records-signature = { $kind }: { $detail }
    서로 다른 실패 실행 { $count }회 · 처음 관찰: { $first } UTC
    식별값: { $key }
records-run-refs = 보관된 실행 참조:
    { $runs }
records-historical-lead = 이전에 관찰됨: 작업 #{ $task }의 과거 해결 기록
    근거가 확보된 진단: { $support }
    이후 통과한 실행: { $run }
    소스 변경·원인: { $cause }
records-lead-only = 과거 해결 기록은 조사 단서일 뿐, 현재 문제의 수정이나 검증을 뜻하지 않습니다.
records-no-signatures = 이 프로젝트에 보관된 실패 식별 기록이 없습니다.
records-resolution-summary = 과거 관계 연결: { $signature } · { $support } · { $run }
    현재 수정 검증은 아닙니다.
records-resolution-choice = 실패 식별값: { $signature }
    근거가 확보된 진단 영수증: { $support }
    이후 명령·경로가 일치하며 통과한 실행: { $run }
    소스 변경·원인: { $cause }
    수동 소스 변경만으로는 누가 수정했는지 알 수 없습니다.
records-no-resolution = 아직 연결 가능한 관계가 없습니다. 유효한 근거를 보존한 진단 영수증, 이후 관찰된 소스 변경, 그 뒤 동일한 명령·작업 경로에서 완료되어 통과한 소유 실행이 필요합니다. 관계를 추측하지 않았습니다.
records-support-receipt = 진단 영수증 { $reference } · { $freshness }
    { $detail }
records-habit-disclosure = 정규화한 작성 지시가 서로 다른 요청에서 실제로 세 번 이상 전송된 경우에만 제안합니다. 승인은 기존 프로젝트 메모리 검토 화면에서 진행하며 자동으로 저장하지 않습니다.
records-habit-hash = 안정적인 지시 해시: { $key }
records-habit-source = 전송된 요청 { $request } · 작업 #{ $task } · { $owner }
records-no-habits = 승인 대기 중인 반복 지시 제안이 없습니다. 거절한 해시는 다시 제안하지 않습니다.
records-habit-memory-disclosure = 실제 전송된 원본 요청: { $sources }
    명시적으로 승인하면 프로젝트 메모리를 저장·선택하고 이 지시와 연결합니다. 이후 현재 지시가 정확히 반복되면 원문을 중복 전송하는 대신 선택된 메모리를 참조합니다. 원래 초안과 과거 대화·제공자 기록은 바뀌지 않습니다. 메모리를 편집·삭제하거나 선택을 해제하면 치환하지 않습니다.
records-pairing-disclosure = 프로젝트·파트너·폼별 로컬 선택입니다. 제공자·모델을 직접 선택하고 소유 작업이 정리된 뒤 저장할 수 있습니다. 기억된 설정은 연결·상태 조회·전송·권한 부여·과금 변경을 하지 않습니다. 사용할 수 없더라도 다른 모델로 대체하지 않습니다. 제거하면 별도로 보관된 전역 기본값으로 돌아갑니다.
records-pairing-offer = 직접 선택한 현재 설정 · 아직 저장되지 않음:
records-pairing-committed = 저장이 확정된 설정:
records-pairing-value = 파트너 #{ $partner } · { $form }
    { $agent } / { $model } / 추론 강도 { $effort }
records-no-pairings = 선택된 저장 설정이 없습니다.
records-default = 기본값
records-remembered = 기억된 설정
records-remembered-unavailable = 기억된 설정 · 사용할 수 없거나 미확인
records-discard-disclosure = 표시된 실패한 로컬 쓰기 작업만 버리고 나머지 대기 작업은 유지합니다. 전송된 지시의 소유권 보류는 이 명시적인 결정 후에만 해제됩니다. 이미 확정된 작업을 되돌리거나 제공자 기록을 지우지 않습니다. 처음에는 유지가 선택됩니다.
records-failed-owner = 작업 #{ $task } · { $operation }
records-failed-instruction = 요청 { $request } · 원래 소유자 { $owner }
    보관 중인 작성 지시:
    { $instruction }
records-op-instruction = 전송된 지시 관찰 기록
records-op-pairing = 로컬 제공자·모델 설정 변경
records-op-flavor = 사실 알림 표시 설정
records-op-dismiss = 반복 지시 제안 거절
records-op-bind = 승인된 메모리 연결
records-op-resolve = 과거 해결 관계
records-op-run = 소유 실행 관찰 기록
records-op-load = 기록 불러오기
records-op-usage = 제공자 사용량 영수증
records-controls = ↑↓ 선택 · PgUp/PgDn 페이지 · Home/End · Esc 돌아가기
records-dex-controls = ↑↓ 실패 선택 · PgUp/PgDn 페이지 · /dex-resolve 관계 검토 · Esc
records-habits-controls = ↑↓ 제안 · /habit-approve 메모리 검토 · /habit-dismiss 거절 · PgUp/PgDn · Esc
records-pairing-controls = ↑↓ 설정 · /save-pairing 저장 · /remove-pairing 제거 · PgUp/PgDn · Esc
records-review-controls = { $stay } / { $action } · ←→ · Enter
    { $review }
    PgUp/PgDn 상세 · Esc 돌아가기
records-discard-action = 미저장 쓰기 버리기
records-discard-selected = [미저장 쓰기 버리기]
records-link-action = 과거 관계 연결
records-link-selected = [과거 관계 연결]
records-summary-visible = 범위·효과 요약 확인됨 · y/n 선택
records-summary-unavailable = 요약을 표시할 수 없어 확인할 수 없습니다
chips-inspect = Enter 확인
chips-remove = Del 제거
chips-hint = Tab/Shift+Tab 첨부 선택 · ←/→ 이동 · Enter 확인 · Del 제거 · Esc 초안 유지
chips-current = 캡처한 소스가 현재 상태와 일치
chips-stale = 캡처한 소스가 오래됨
chips-unavailable = 캡처 사용 불가 · 전송 차단
chips-unchecked = 최신 상태 미확인
chips-wire-pending = 정확한 전송 영수증 계산 대기
chips-failure = 실패 위치
chips-changed = 변경된 파일
chips-tests = 테스트 파일
chips-changed-tests = 변경된 테스트
records-remove-pairing = 파트너 #{ $partner }의 { $form } 설정을 제거하고 전역 기본값으로 돌아갑니다.
records-flavor-enabled = 저장이 확정되면 사실 알림을 표시합니다.
records-flavor-disabled = 저장이 확정되면 사실 알림을 끕니다. 이후 이벤트는 쌓아 두지 않고 소비합니다.
records-memory-binding = 지시 해시 { $key }를 승인된 메모리 #{ $memory }의 리비전 { $revision }에 연결합니다.
records-usage-owner = 제공자 사용량 요청 { $request } · 원래 소유자 { $owner }
records-habit-available = /habits — 반복해서 작성·전송한 지시가 있습니다. 명시적인 메모리 승인 검토를 열 수 있습니다.
records-help-summary = 실제 파트너 이력, 공유 유대 메모리와 폼별 산출물을 확인합니다.
records-help-routes = 로컬 폼 전환 일지를 확인합니다. 이 기록은 증거가 아닙니다.
records-help-dex = 현재 프로젝트의 실패 식별값과 보관된 실행 참조를 확인합니다.
records-help-resolve = 과거 진단·소스 변경·통과 실행의 관계를 명시적으로 검토합니다. 현재 문제가 수정되었다는 뜻은 아닙니다.
records-help-habits = 서로 다른 요청에서 실제로 세 번 이상 전송된 동일한 작성 지시를 검토합니다.
records-help-habit-approve = 선택한 반복 지시의 프로젝트 메모리 승인 화면을 엽니다. 자동으로 저장하지 않습니다.
records-help-habit-dismiss = 선택한 지시의 정확한 해시를 기준으로 제안 거절을 저장합니다.
records-help-pairings = 프로젝트·파트너·폼별 제공자와 모델 설정을 확인합니다. 연결하거나 요청하지 않습니다.
records-help-pairing-save = 직접 선택한 현재 제공자·모델·추론 강도를 이 파트너와 폼의 설정으로 저장합니다.
records-help-pairing-remove = 이 폼의 기억된 설정을 제거하고 별도로 보관된 전역 기본값으로 돌아갑니다.
records-help-flavor-off = 사실 알림 표시를 계속 끕니다. 이벤트는 소비하되 나중에 다시 표시하지 않습니다.
records-help-flavor-on = 앞으로 발생하는 사실 알림을 표시합니다. 지난 이벤트는 다시 표시하지 않습니다.
records-help-retry = 보관 중인 실패한 기록 쓰기를 그대로 재시도하거나, 읽기 실패 후 명시적으로 다시 불러옵니다.
records-help-discard = 보관 중인 실패한 로컬 쓰기만 버릴지 검토합니다. 처음에는 유지가 선택됩니다.
chips-focus = Tab 파일 선택
records-write-failed = 로컬 기록 저장 실패 /records-retry 재시도 /records-discard 검토
records-pairing-use-offer = /save-pairing · 여기서 { $partner } 설정으로 사용
records-discard-summary = 미저장 쓰기 버리기: { $identity }
    확정 작업과 다른 대기 쓰기는 유지합니다.
records-compact-marker = { $status } · 산출물 { $count }개 · { $producer }
records-project-setting = 현재 프로젝트의 로컬 설정
ribbons-title = 리본 / 프로젝트 이력
ribbons-history = 과거 기록 / 아래에서 선택한 근거 확인
ribbons-disclosure = 처음 기록된 적격 사건이며 현재의 정확성을 뜻하지 않습니다. 로컬 기록만 표시하며 권한을 주거나 작업을 시작하지 않습니다.
ribbons-empty = 이 프로젝트에 기록된 적격 이정표 이력이 없습니다.
ribbons-accepted-review = 유효한 승인 검토 기록
ribbons-verified-handoff = 검증된 인계 기록
ribbons-learned-move = 명시적으로 배운 기술 기록
ribbons-line = { $kind } / { $name } / { $status }
ribbons-details = 작업 #{ $task } / 생성자 #{ $partner } { $name } / { $form }
    기록 날짜 { $date } UTC
    현재 근거: { $status }
    { $reason }
ribbons-hidden = 파트너 요약에서 숨김. 근거는 유지합니다.
ribbons-visible = 로컬 표현을 켰을 때 파트너 요약에 표시합니다.
ribbons-foreign-task = 다른 원본 작업입니다. Enter는 작업 목록만 엽니다. 재개하려면 직접 선택하세요. 현재 작업은 바뀌지 않습니다.
ribbons-artifact = 보관된 산출물: { $id }
ribbons-move = 원본 기술 #{ $id } / 개정 { $revision }
ribbons-foreign-owner = 생성자 또는 폼이 다릅니다. Enter는 일반 파티/폼 선택기만 열며 선두나 폼을 바꾸지 않습니다.
ribbons-open-artifact = 이미 원본 작업에 있을 때 Enter는 일반 검토/인계 확인 화면을 엽니다. 내보내거나 전송하지 않습니다.
ribbons-open-move = Enter는 현재 소유자/폼의 일반 기술 상세/목록만 엽니다. 삭제된 원본을 대체하지 않습니다.
ribbons-controls = Up/Down 선택 / h 숨김·표시 / Enter 근거
    PgUp/PgDn 페이지 / Home/End / Esc 뒤로
ribbons-visibility = 과거 기록 표시 설정
ribbons-visibility-detail = { $kind } / 숨김 = { $hidden }
    로컬 요약 표시만 바꾸며 근거는 삭제하지 않습니다.
ribbons-help = 숨긴 기록과 현재 원본 상태를 포함한 프로젝트 이정표 이력을 확인합니다.
ribbons-help-hide = 선택한 리본을 자동 요약에서 숨기고 과거 근거는 유지합니다.
ribbons-help-show = 로컬 표현이 켜져 있을 때 선택한 리본을 파트너 요약에 표시합니다.
ribbons-help-inspect = 일반 로컬 탐색으로 선택한 근거를 엽니다. 작업을 자동 재개하거나 생성자를 바꾸지 않습니다.
ribbons-source-unavailable = 원본을 사용할 수 없거나 불명확합니다. 과거 상세는 유지하며 다른 원본을 열지 않았습니다.
ribbons-hidden-tag = [숨김]{ " " }
ribbons-source-evidence = 소스 / 작업 #{ $task } / 캡처 #{ $excerpt } / { $path }:{ $start }-{ $end }
ribbons-run-evidence = 실행 / 작업 #{ $task } / { $run }
