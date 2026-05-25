mod crypto;
mod sagemaker;
mod attestation;
mod purge;

use std::sync::Arc;
use axum::{routing::post, Router, extract::State, Json};
use axum_server::tls_rustls::RustlsConfig;
use rustls::ServerConfig;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub config: ProxyConfig,
    pub kms_client: aws_sdk_kms::Client,
    pub s3_client: aws_sdk_s3::Client,
    pub sm_client: aws_sdk_sagemaker::Client,
    pub attestor: attestation::NitroAttestor,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProxyConfig {
    pub magalu_kms_key_id: String,
    pub aws_role_arn: String,
    pub ephemeral_bucket: String,
    pub max_residence_secs: u64,
}

#[derive(Deserialize)]
pub struct TrainRequest {
    pub training_data_uri: String,      // encrypted data path in Magalu Object Storage
    pub algorithm: String,
    pub instance_type: String,
    pub hyperparameters: serde_json::Value,
}

#[derive(Serialize)]
pub struct TrainResponse {
    pub job_name: String,
    pub model_uri: String,
    pub residence_secs: u64,
    pub seal: String,
}

async fn train_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<TrainRequest>,
) -> Result<Json<TrainResponse>, (axum::http::StatusCode, String)> {
    info!("Training request received");

    // 1. Verify attestation (Nitro Enclave)
    if !state.attestor.verify_attestation().await {
        return Err((axum::http::StatusCode::FORBIDDEN, "Attestation failed".to_string()));
    }

    // 2. Generate session key
    let session_key = crypto::generate_session_key();

    // 3. Fetch encrypted data from Magalu OS
    let encrypted_data = crypto::fetch_from_magalu(&payload.training_data_uri).await
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;

    // 4. Re-encrypt with session key
    let (ciphertext, nonce) = crypto::envelope_encrypt(&encrypted_data, &session_key)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 5. Upload to ephemeral S3
    let job_id = uuid::Uuid::new_v4().to_string();
    let input_key = format!("ephemeral/{}/train.enc", job_id);
    sagemaker::upload_to_s3(&state.s3_client, &state.config.ephemeral_bucket, &input_key, &ciphertext).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 6. Create SageMaker training job
    let job_name = format!("arkhe-train-{}", job_id);
    sagemaker::create_training_job(&state.sm_client, &state.config, &job_name, &input_key, &payload).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 7. Poll until completion
    let start = std::time::Instant::now();
    let mut model_artifact_uri = None;
    loop {
        let status = sagemaker::describe_job(&state.sm_client, &job_name).await
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if status == "Completed" {
            model_artifact_uri = Some(sagemaker::get_model_artifact(&state.sm_client, &job_name).await
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?);
            break;
        } else if status == "Failed" || status == "Stopped" {
            // Purge immediately on failure
            purge::purge_ephemeral(&state.s3_client, &state.config.ephemeral_bucket, &job_id).await;
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Training job {}", status)));
        }
        if start.elapsed().as_secs() > state.config.max_residence_secs {
            sagemaker::stop_job(&state.sm_client, &job_name).await;
            purge::purge_ephemeral(&state.s3_client, &state.config.ephemeral_bucket, &job_id).await;
            return Err((axum::http::StatusCode::GATEWAY_TIMEOUT, "Residence time exceeded".to_string()));
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }

    let _artifact_uri = model_artifact_uri.unwrap();
    let residence = start.elapsed().as_secs();

    // 8. Download encrypted model
    let encrypted_model = sagemaker::download_from_s3(&state.s3_client, &state.config.ephemeral_bucket, &format!("ephemeral/{}/output/model.tar.gz", job_id)).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 9. Decrypt with session key
    let model_plain = crypto::decrypt(&encrypted_model, &session_key, &nonce)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 10. Save to Magalu Object Storage (canonical origin)
    let model_uri = format!("{}/models/{}/model.tar.gz", state.config.magalu_kms_key_id, job_id);
    crypto::upload_to_magalu(&model_uri, &model_plain).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 11. Purge ephemeral data
    purge::purge_ephemeral(&state.s3_client, &state.config.ephemeral_bucket, &job_id).await;

    // 12. Compute seal
    let seal = crypto::compute_seal(&job_name, &model_uri, residence);

    // 13. Revoke session key (zeroize)
    drop(session_key);

    Ok(Json(TrainResponse {
        job_name,
        model_uri,
        residence_secs: residence,
        seal,
    }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let proxy_config = ProxyConfig {
        magalu_kms_key_id: std::env::var("MAGALU_KMS_KEY_ID")?,
        aws_role_arn: std::env::var("AWS_ROLE_ARN")?,
        ephemeral_bucket: std::env::var("AWS_EPHEMERAL_BUCKET")?,
        max_residence_secs: std::env::var("MAX_RESIDENCE_SECS")?.parse()?,
    };

    let state = Arc::new(AppState {
        config: proxy_config,
        kms_client: aws_sdk_kms::Client::new(&config),
        s3_client: aws_sdk_s3::Client::new(&config),
        sm_client: aws_sdk_sagemaker::Client::new(&config),
        attestor: attestation::NitroAttestor::new(&config).await,
    });

    // mTLS configuration
    let cert = rustls_pemfile::certs(&mut std::fs::read("/etc/arkhe/tls/server.crt")?.as_slice()).collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut std::fs::read("/etc/arkhe/tls/server.key")?.as_slice())?.unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder(load_client_ca()?.into()).build()?;
    let server_config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert, key)?;
    let tls_config = RustlsConfig::from_config(Arc::new(server_config));

    let app = Router::new()
        .route("/v1/sagemaker/train", post(train_handler))
        .with_state(state);

    info!("SageMaker Proxy starting on 0.0.0.0:8242 with mTLS");
    axum_server::tls_rustls::bind_rustls("0.0.0.0:8242".parse()?, tls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn load_client_ca() -> Result<rustls::RootCertStore, Box<dyn std::error::Error>> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_cert = std::fs::read("/etc/arkhe/tls/ca.crt")?;
    let ca_cert = rustls_pemfile::certs(&mut ca_cert.as_slice()).collect::<Vec<_>>();
    for cert in ca_cert {
        root_store.add(cert?)?;
    }
    Ok(root_store)
}