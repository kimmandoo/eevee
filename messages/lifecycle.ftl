lifecycle-command-update = Review a checksum-verified application update; nothing replaced yet
lifecycle-command-rollback = Review a retained package; rollback requires proven data compatibility
lifecycle-update-title = Application update · review before replacing
lifecycle-rollback-title = Retained package rollback · compatibility required
lifecycle-waiting = Waiting for committed drafts, local saves and idle work. No install has started. Finish pending work; r checks again, Esc returns.
lifecycle-expired = Review expired after navigation, task or local-state change. Press r for a fresh check; nothing approved.
lifecycle-unavailable = Update/rollback unavailable; installation and task data were not changed by this check.
lifecycle-failed = Installation helper failed. Inspect its terminal output and recovery details; do not assume replacement or rollback succeeded. Press r to inspect the current installation.
lifecycle-update-cache = Existing sprite cache is retained. The installer may download missing verified sprites; completed cache downloads may remain even if installation fails.
lifecycle-rollback-cache = Offline retained package only: no network request or sprite-cache change.
lifecycle-update-data = Installer does not replace task/provider data. Back up the ENTIRE data directory first. The new release may migrate supported data when reopened. Saved task/draft resumes; sessions and grants do not.
lifecycle-rollback-data = Exact durable-format identity and current read-only data validation are required; equal SQL schema numbers are not proof. No task-data rewrite or backup restoration. Saved task/draft resumes without sessions or grants.
lifecycle-scope = Current: { $current }
    Target: { $target }
    Source: { $source }
    Installation: { $directory }
    { "" }
    Cache: { $cache }
    { "" }
    Data / resume: { $data }
    { "" }
    Eligibility: { $reason }
    { "" }
    One explicit replacement only. Existing ownership/checksum and failed-publication recovery checks remain required. No elevation or shell-profile edits.
lifecycle-choose-stay = [Stay] / Apply once · Tab/Right choose · Enter stays
    y apply once · n/Esc return · r fresh check
lifecycle-choose-apply = Stay / [Apply once] · Enter applies this exact review
    y apply once · n/Esc return · r fresh check
lifecycle-inspect = PgUp/PgDn inspect the COMPLETE scope before approval.
    Enter/n/Esc returns without installation · r fresh check
