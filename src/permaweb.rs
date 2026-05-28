use chrono::{Utc, DateTime};
use log::{info, warn};
use serde_json::{json, Value};
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct PermawebConfig {
    pub arweave_gateway: String,
    pub arweave_graphql: String,
    pub ao_mu_url: String,
    pub ao_cu_url: String,
    pub ao_su_url: String,
    pub aos_process_id: Option<String>,
    pub aos_module_id: String,
    pub wallet_path: Option<String>,
    pub auto_upload: bool,
    pub upload_interval: u64,
    pub hyperbeam_endpoint: Option<String>,
    pub debug: bool,
}

impl Default for PermawebConfig {
    fn default() -> Self {
        Self {
            arweave_gateway: "https://arweave.net".to_string(),
            arweave_graphql: "https://arweave.net/graphql".to_string(),
            ao_mu_url: "https://mu.ao-testnet.xyz".to_string(),
            ao_cu_url: "https://cu.ao-testnet.xyz".to_string(),
            ao_su_url: "https://su.ao-testnet.xyz".to_string(),
            aos_process_id: None,
            aos_module_id: "SBNb1qPQ1TDwpD_mokmNSia-Sc-7tPvUl8keTt22PE".to_string(),
            wallet_path: None,
            auto_upload: false,
            upload_interval: 3600,
            hyperbeam_endpoint: None,
            debug: false,
        }
    }
}

pub struct ArweaveDataLayer {
    pub config: PermawebConfig,
}

impl ArweaveDataLayer {
    pub fn new(config: PermawebConfig) -> Self {
        Self { config }
    }

    pub fn upload_data(&self, data: &str, tags: Option<HashMap<String, String>>) -> Value {
        let mut hasher = Sha3_256::new();
        hasher.update(data.as_bytes());
        let mock_tx_id = hex::encode(hasher.finalize());

        let mut final_tags = HashMap::new();
        final_tags.insert("App-Name".to_string(), "ARKHE-OS".to_string());
        final_tags.insert("App-Version".to_string(), "2.0.0".to_string());
        final_tags.insert("Substrate".to_string(), "927".to_string());
        final_tags.insert("Content-Type".to_string(), "application/json".to_string());

        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
        final_tags.insert("Timestamp".to_string(), ts);

        if let Some(t) = tags {
            for (k, v) in t {
                final_tags.insert(k, v);
            }
        }

        json!({
            "tx_id": mock_tx_id,
            "status": "mock_uploaded",
            "url": format!("{}/{}", self.config.arweave_gateway, mock_tx_id),
            "size": data.len(),
            "tags": final_tags,
            "mock": true,
        })
    }

    pub fn fetch_data(&self, tx_id: &str) -> Value {
        json!({
            "tx_id": tx_id,
            "error": "Fetch not implemented without networking",
            "status": "failed",
        })
    }

    pub fn query_transactions(&self, _tags: HashMap<String, String>, _limit: usize) -> Vec<Value> {
        vec![]
    }
}

pub struct AOComputerInterface {
    pub config: PermawebConfig,
}

impl AOComputerInterface {
    pub fn new(config: PermawebConfig) -> Self {
        Self { config }
    }

    pub fn spawn_process(&self, _module_id: Option<String>, _tags: Option<Vec<HashMap<String, String>>>) -> Value {
        json!({
            "id": format!("ao-process-mock-{}", Utc::now().timestamp_millis()),
            "status": "spawned"
        })
    }

    pub fn send_message(&self, process_id: &str, action: &str, _data: Option<&str>, _tags: Option<HashMap<String, String>>) -> Value {
        json!({
            "message_id": format!("msg-mock-{}-{}", process_id, action),
            "status": "sent"
        })
    }
}

pub struct AOSInterface {
    pub process_id: Option<String>,
}

impl AOSInterface {
    pub fn new(config: &PermawebConfig) -> Self {
        Self { process_id: config.aos_process_id.clone() }
    }

    pub fn spawn_aos(&mut self, _name: &str) -> Value {
        let new_id = format!("aos-mock-{}", Utc::now().timestamp_millis());
        self.process_id = Some(new_id.clone());
        json!({
            "id": new_id,
            "status": "spawned"
        })
    }

    pub fn load_blueprint(&self, _blueprint_name: &str) -> Value {
        json!({"status": "loaded"})
    }

    pub fn eval_lua(&self, _lua_code: &str) -> Value {
        json!({"status": "evaluated"})
    }
}

pub struct HyperBEAMInterface {
    pub config: PermawebConfig,
}

impl HyperBEAMInterface {
    pub fn new(config: PermawebConfig) -> Self {
        Self { config }
    }

    pub fn resolve_path(&self, _path: &str) -> Value {
        json!({"error": "HyperBEAM endpoint not configured"})
    }

    pub fn execute_device(&self, device_name: &str, input_data: Value) -> Value {
        json!({
            "device": device_name,
            "input": input_data,
            "status": "executed",
            "mock": true,
        })
    }
}

pub struct PermawebBridge {
    pub config: PermawebConfig,
    pub arweave: ArweaveDataLayer,
    pub ao: AOComputerInterface,
    pub aos: AOSInterface,
    pub hyperbeam: HyperBEAMInterface,
    upload_history: Vec<Value>,
    process_registry: HashMap<String, Value>,
}

impl PermawebBridge {
    pub fn new(config: PermawebConfig) -> Self {
        let arweave = ArweaveDataLayer::new(config.clone());
        let ao = AOComputerInterface::new(config.clone());
        let aos = AOSInterface::new(&config);
        let hyperbeam = HyperBEAMInterface::new(config.clone());

        Self {
            config,
            arweave,
            ao,
            aos,
            hyperbeam,
            upload_history: Vec::new(),
            process_registry: HashMap::new(),
        }
    }

    pub fn persist_agent_state(&mut self, agent_state: Value, agent_id: &str) -> Value {
        let state_json = agent_state.to_string();
        let mut tags = HashMap::new();
        tags.insert("Agent-ID".to_string(), agent_id.to_string());
        tags.insert("Substrate".to_string(), "927".to_string());
        tags.insert("Type".to_string(), "Agent-State".to_string());
        tags.insert("Version".to_string(), "2.0.0".to_string());
        tags.insert("Timestamp".to_string(), Utc::now().to_rfc3339());

        let result = self.arweave.upload_data(&state_json, Some(tags));
        self.upload_history.push(result.clone());
        result
    }

    pub fn spawn_agent_process(&mut self, agent_id: &str, name: &str) -> Value {
        let mut t1 = HashMap::new();
        t1.insert("name".to_string(), "Agent-ID".to_string());
        t1.insert("value".to_string(), agent_id.to_string());

        let mut t2 = HashMap::new();
        t2.insert("name".to_string(), "Name".to_string());
        t2.insert("value".to_string(), name.to_string());

        let mut t3 = HashMap::new();
        t3.insert("name".to_string(), "Substrate".to_string());
        t3.insert("value".to_string(), "927".to_string());

        let tags = vec![t1, t2, t3];
        let result = self.ao.spawn_process(None, Some(tags));

        if let Some(id) = result.get("id").and_then(|i| i.as_str()) {
            self.process_registry.insert(agent_id.to_string(), json!({
                "process_id": id,
                "created_at": Utc::now().to_rfc3339()
            }));
        }

        result
    }

    pub fn get_bridge_status(&self) -> Value {
        json!({
            "substrate": "927",
            "arweave_gateway": self.config.arweave_gateway,
            "ao_mu": self.config.ao_mu_url,
            "ao_cu": self.config.ao_cu_url,
            "aos_process": self.config.aos_process_id,
            "hyperbeam_endpoint": self.config.hyperbeam_endpoint,
            "uploads_count": self.upload_history.len(),
            "processes_registered": self.process_registry.len(),
            "wallet_configured": self.config.wallet_path.is_some(),
        })
    }
}
