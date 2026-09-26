# Pass195 implementation report

Pass195 begins the pre-split hostile cleanup of `kernel-query`.

Closed in this wave:

1. indexed generic Group deltas no longer perform accidental all-groups x all-delta O(d^2) planning;
2. standalone Group publication now uses the same sealed, total commit boundary as the maintained flat plan;
3. canonical AntiJoin left-removal batches are processed by full row class in one bucket pass rather than repeated linear search/removal;
4. canonical Set insertion duplicate validation uses ordered-set membership instead of a growing linear Vec scan.

Only `crates/kernel-query/src/lib.rs` changed in production. The monolith remains intentionally unsplit because the global hostile audit is still OPEN; Pass196 should continue the hostile list in `PASS195_REPORT.md` rather than begin mechanical extraction.
