use aws_sdk_s3::Client as S3Client;

pub async fn purge_ephemeral(s3: &S3Client, bucket: &str, job_id: &str) {
    let prefix = format!("ephemeral/{}/", job_id);
    let list = s3.list_objects_v2()
        .bucket(bucket)
        .prefix(&prefix)
        .send()
        .await;

    if let Ok(resp) = list {
        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                let _ = s3.delete_object()
                    .bucket(bucket)
                    .key(key)
                    .send()
                    .await;
            }
        }
    }
    tracing::info!("Purged ephemeral data for job {}", job_id);
}