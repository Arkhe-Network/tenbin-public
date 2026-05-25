use aws_sdk_sagemaker::{Client as SmClient, types};
use aws_sdk_s3::Client as S3Client;
use super::ProxyConfig;
use super::TrainRequest;

pub async fn create_training_job(
    sm: &SmClient,
    config: &ProxyConfig,
    job_name: &str,
    input_key: &str,
    req: &TrainRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let input_data = format!("s3://{}/{}", config.ephemeral_bucket, input_key);
    let output_path = format!("s3://{}/ephemeral/{}/output", config.ephemeral_bucket, job_name);

    sm.create_training_job()
        .training_job_name(job_name)
        .role_arn(&config.aws_role_arn)
        .algorithm_specification(
            types::AlgorithmSpecification::builder()
                .training_image(training_image_for(&req.algorithm))
                .training_input_mode(types::TrainingInputMode::File)
                .build()
        )
        .input_data_config(
            types::Channel::builder()
                .channel_name("train")
                .data_source(
                    types::DataSource::builder()
                        .s3_data_source(
                            types::S3DataSource::builder()
                                .s3_data_type(types::S3DataType::S3Prefix)
                                .s3_uri(&input_data)
                                .build()
                        )
                        .build()
                )
                .build()
        )
        .output_data_config(
            types::OutputDataConfig::builder()
                .s3_output_path(&output_path)
                .build()
        )
        .resource_config(
            types::ResourceConfig::builder()
                .instance_type(types::TrainingInstanceType::from(req.instance_type.as_str()))
                .instance_count(1)
                .volume_size_in_gb(30)
                .build()
        )
        .stopping_condition(
            types::StoppingCondition::builder()
                .max_runtime_in_seconds(config.max_residence_secs as i32)
                .build()
        )
        .set_hyper_parameters(Some(req.hyperparameters.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()))
        .send()
        .await?;

    Ok(())
}

pub async fn describe_job(sm: &SmClient, job_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let resp = sm.describe_training_job()
        .training_job_name(job_name)
        .send()
        .await?;
    Ok(resp.training_job_status.unwrap().as_str().to_string())
}

pub async fn get_model_artifact(sm: &SmClient, job_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let resp = sm.describe_training_job()
        .training_job_name(job_name)
        .send()
        .await?;
    Ok(resp.model_artifacts.unwrap().s3_model_artifacts.unwrap())
}

pub async fn stop_job(sm: &SmClient, job_name: &str) {
    let _ = sm.stop_training_job()
        .training_job_name(job_name)
        .send()
        .await;
}

pub async fn upload_to_s3(s3: &S3Client, bucket: &str, key: &str, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    s3.put_object()
        .bucket(bucket)
        .key(key)
        .body(aws_sdk_s3::primitives::ByteStream::from(data.to_vec()))
        .send()
        .await?;
    Ok(())
}

pub async fn download_from_s3(s3: &S3Client, bucket: &str, key: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let resp = s3.get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await?;
    Ok(resp.body.collect().await?.into_bytes().to_vec())
}

fn training_image_for(algorithm: &str) -> &str {
    match algorithm {
        "xgboost" => "683313688378.dkr.ecr.us-east-1.amazonaws.com/sagemaker-xgboost:1.5-1",
        "linear-learner" => "382416733822.dkr.ecr.us-east-1.amazonaws.com/linear-learner:latest",
        _ => "382416733822.dkr.ecr.us-east-1.amazonaws.com/sagemaker-scikit-learn:0.23-1-cpu-py3",
    }
}