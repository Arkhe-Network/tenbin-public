use serde_json::Value;
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct OctraService {
    pub fhe_keys: HashMap<String, HashMap<String, Value>>,
    pub zk_domains: HashMap<String, (u32, u32)>,
    pub pqc_registry: HashMap<String, HashMap<String, Value>>,
    pub store: HashMap<String, HashMap<String, Value>>,
    pub log: Vec<String>,
}

impl OctraService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provision_fhe(&mut self, pk_id: &str, levels: u32) -> HashMap<String, Value> {
        let mut props = HashMap::new();
        props.insert("levels".to_string(), Value::Number(levels.into()));
        self.fhe_keys.insert(pk_id.to_string(), props);

        let mut result = HashMap::new();
        result.insert("pk_id".to_string(), Value::String(pk_id.to_string()));
        result
    }

    pub fn encrypt_fhe(&mut self, pk_id: &str, vec: Vec<f64>) -> HashMap<String, Value> {
        let vec_str = format!("{:?}", vec);
        let mut hasher = Sha3_256::new();
        hasher.update(vec_str.as_bytes());
        let h = hex::encode(hasher.finalize())[..16].to_string();

        let mut data = HashMap::new();
        data.insert("data".to_string(), Value::String(vec_str));
        if let Some(key) = self.fhe_keys.get(pk_id) {
            data.insert("level".to_string(), key.get("levels").unwrap().clone());
        }
        self.store.insert(h.clone(), data);

        let mut result = HashMap::new();
        result.insert("handle".to_string(), Value::String(h));
        result
    }

    pub fn prove_zk(&mut self, _domain: &str, secret: &str, challenge: u32) -> HashMap<String, Value> {
        let mut hasher = Sha3_256::new();
        hasher.update(format!("{}{}", secret, challenge).as_bytes());
        let proof_id = hex::encode(hasher.finalize())[..16].to_string();

        let mut result = HashMap::new();
        result.insert("proof_id".to_string(), Value::String(proof_id));
        result
    }

    pub fn sign_pqc(&mut self, eid: &str, msg: &str) -> HashMap<String, Value> {
        let mut hasher = Sha3_256::new();
        hasher.update(format!("{}{}", eid, msg).as_bytes());
        let signature = hex::encode(hasher.finalize())[..32].to_string();

        let mut result = HashMap::new();
        result.insert("signature".to_string(), Value::String(signature));
        result
    }

    pub fn provision_pqc(&mut self, eid: &str, level: u32) -> HashMap<String, Value> {
        let mut props = HashMap::new();
        props.insert("level".to_string(), Value::Number(level.into()));
        self.pqc_registry.insert(eid.to_string(), props);

        let mut result = HashMap::new();
        result.insert("entity_id".to_string(), Value::String(eid.to_string()));
        result
    }

    pub fn provision_zk(&mut self, domain: &str, g: u32, h: u32) -> HashMap<String, Value> {
        self.zk_domains.insert(domain.to_string(), (g, h));

        let mut result = HashMap::new();
        result.insert("domain".to_string(), Value::String(domain.to_string()));
        result
    }
}
