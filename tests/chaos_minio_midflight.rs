//! Chaos: image upload mid-flight failure.
//!
//! Phase 4.7 skeleton. Pairs with `TESTING.md` § 9 last row.
//!
//! Asserted behavior: client gets 5xx; no partial creative row is
//! committed (the upload handler must finalize the object in the
//! object store before recording `image_url` on the creative
//! row).
//!
//! Injection: kill MinIO container during a multi-part upload.

#[tokio::test]
#[ignore = "chaos suite — needs the compose harness with a MinIO container the test can `docker kill` mid-upload. Activate by flipping #[ignore] once the harness lands."]
async fn minio_killed_midflight() {
    // 1. compose up with MinIO + knievel
    // 2. start a multi-part image upload to /v1/projects/{p}/
    //    creatives/{id}/image
    // 3. midway through the upload (after the first part lands
    //    in MinIO), `docker compose kill knievel-minio`
    // 4. assert: client receives 5xx
    // 5. assert: GET /v1/projects/{p}/creatives/{id} shows
    //    image_url unchanged (no partial row was committed)
    // 6. cleanup: `docker compose start knievel-minio` and
    //    confirm a retry succeeds end-to-end
}
