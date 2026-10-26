diagnostics-status-blocked = Needs attention

diagnostics-missing = Executable missing on the observed PATH.
diagnostics-authentication = Authentication required: provider reported signed out or HTTP 401.
diagnostics-eligibility = HTTP 403: access refused; login status is unknown.
diagnostics-endpoint = Endpoint or model unavailable; not authentication proof.
diagnostics-rate-limit = Provider quota/rate limit reached; no automatic retry.
diagnostics-request = Provider rejected the request/model/effort settings.
diagnostics-transport = Transport failed; authentication and remote outcome unknown.
diagnostics-incompatible = Protocol/policy capability mismatch; request stopped.
diagnostics-version-check = Version check failed; compatibility and login unknown.
diagnostics-login-cancelled = Login cancelled; provider changes may already exist.
diagnostics-login-expired = Login approval/operation expired; not authentication proof.
diagnostics-login-failed = Login/status failed; private output was not retained.
diagnostics-login-refused = Login launch refused: reviewed prerequisites changed.
diagnostics-unknown = Installation/configuration is not authentication or eligibility proof.

diagnostics-next-install = n: Setup instructions; install/update, then Refresh.
diagnostics-next-login = n: Review login/setup again; no request is replayed.
diagnostics-next-account = n: Review account login; check provider workspace policy.
diagnostics-next-api = n: Review this route's key/endpoint; no billing switch.
diagnostics-next-settings = n: Review model settings and provider access/quota.
diagnostics-next-setup = n: Inspect local setup/connectivity; retry only explicitly.
diagnostics-next-claude = n: Console/API setup; claude.ai login is not SDK auth.
diagnostics-next-gemini = n: Official Gemini login review; Eevee collects no code.
diagnostics-next-devin = n: Review Devin login; eligibility remains provider-owned.

diagnostics-navigation = Enter/n next step · ↑↓ scroll · Tab providers · Esc back
diagnostics-actions-navigation = ↑↓ actions · Enter open · n next · d details · Esc back
diagnostics-account-navigation = Enter login/setup · Tab choose · d details · Esc back
diagnostics-method-navigation = ↑↓ method · Enter review · d details · Esc back
diagnostics-detail-boundary = Provider failures retain status/category only, never response bodies, headers, codes or login output. Local prerequisite details are bounded and best-effort redacted. Setup, version checks and model catalogues do not prove authentication or account/model eligibility. No automatic send, retry, install or billing change.
diagnostics-detail-http = Observed HTTP status: { $status }. Response body and headers were not retained.
diagnostics-detail-local = Local observation: { $category }. No raw authentication output was retained.
diagnostics-detail-preparation = Redacted local prerequisite detail: { $detail }
