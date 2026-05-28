use chrono::Utc;
use serde_json::Value;
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;

use crate::crypto::OctraService;

#[derive(Clone, Debug)]
pub struct Vertex {
    pub vid: String,
    pub vtype: String,
    pub properties: HashMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct Hyperedge {
    pub eid: String,
    pub etype: String,
    pub vertices: Vec<String>,
    pub properties: HashMap<String, Value>,
}

pub struct HypergraphRegistry {
    pub endpoint: String,
    pub vertices: HashMap<String, Vertex>,
    pub edges: HashMap<String, Hyperedge>,
}

impl HypergraphRegistry {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            vertices: HashMap::new(),
            edges: HashMap::new(),
        }
    }

    pub fn add_vertex(&mut self, v: Vertex) {
        self.vertices.insert(v.vid.clone(), v);
    }

    pub fn add_hyperedge(&mut self, e: Hyperedge) {
        self.edges.insert(e.eid.clone(), e);
    }
}

pub struct MemorySpace {
    pub agent_id: String,
    pub entries: Vec<HashMap<String, Value>>,
}

impl MemorySpace {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, entry: HashMap<String, Value>) {
        self.entries.push(entry);
    }

    pub fn retrieve_relevant(&self, query: &str) -> Vec<HashMap<String, Value>> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                if let Some(content) = e.get("content") {
                    let content_str = match content {
                        Value::String(s) => s.clone(),
                        _ => content.to_string(),
                    };
                    content_str.to_lowercase().contains(&q)
                } else {
                    false
                }
            })
            .cloned()
            .collect()
    }
}

pub struct EncryptedMemoryCommit<'a> {
    pub octra: &'a mut OctraService,
    pub agent_id: String,
    pub fhe_pk: String,
    pub zk_domain: String,
    pub pqc_entity: String,
}

impl<'a> EncryptedMemoryCommit<'a> {
    pub fn new(
        octra: &'a mut OctraService,
        agent_id: &str,
        fhe_pk: &str,
        zk_domain: &str,
        pqc_entity: &str,
    ) -> Self {
        Self {
            octra,
            agent_id: agent_id.to_string(),
            fhe_pk: fhe_pk.to_string(),
            zk_domain: zk_domain.to_string(),
            pqc_entity: pqc_entity.to_string(),
        }
    }

    pub fn commit(&mut self, memory_id: &str, payload: &HashMap<String, Value>) -> HashMap<String, Value> {
        let payload_str = serde_json::to_string(payload).unwrap_or_default();
        let vec: Vec<f64> = payload_str.chars().take(100).map(|c| c as u32 as f64).collect();

        let fhe_handle = self.octra.encrypt_fhe(&self.fhe_pk, vec);
        let proof = self.octra.prove_zk(&self.zk_domain, "memory_seed", 42);
        let msg = format!("{}{}", fhe_handle.get("handle").unwrap().as_str().unwrap(), proof.get("proof_id").unwrap().as_str().unwrap());
        let sig = self.octra.sign_pqc(&self.pqc_entity, &msg);

        let mut artefact = HashMap::new();
        artefact.insert("type".to_string(), Value::String("memory.commit".to_string()));
        artefact.insert("agent".to_string(), Value::String(self.agent_id.clone()));
        artefact.insert("memory_id".to_string(), Value::String(memory_id.to_string()));
        artefact.insert("fhe_handle".to_string(), fhe_handle.get("handle").unwrap().clone());
        artefact.insert("zk_proof_id".to_string(), proof.get("proof_id").unwrap().clone());
        artefact.insert("pqc_signature".to_string(), sig.get("signature").unwrap().clone());
        artefact.insert("timestamp".to_string(), Value::String(Utc::now().to_rfc3339()));

        let mut hasher = Sha3_256::new();
        hasher.update(serde_json::to_string(&artefact).unwrap().as_bytes());
        let seal = hex::encode(hasher.finalize());
        artefact.insert("seal".to_string(), Value::String(seal));

        artefact
    }
}

pub struct EpistemicCommitProtocol<'a> {
    pub memory: &'a mut MemorySpace,
    pub committer: EncryptedMemoryCommit<'a>,
    pub hg: &'a mut HypergraphRegistry,
    pub agent_v: Vertex,
}

impl<'a> EpistemicCommitProtocol<'a> {
    pub fn new(
        memory: &'a mut MemorySpace,
        committer: EncryptedMemoryCommit<'a>,
        hg: &'a mut HypergraphRegistry,
        agent_v: Vertex,
    ) -> Self {
        Self {
            memory,
            committer,
            hg,
            agent_v,
        }
    }

    pub fn commit(&mut self, content: HashMap<String, Value>, _relevance: f32, _sensitivity: f32) -> String {
        let content_str = serde_json::to_string(&content).unwrap_or_default();
        let mut hasher = Sha3_256::new();
        hasher.update(content_str.as_bytes());
        let cid = hex::encode(hasher.finalize())[..16].to_string();

        let mut entry = HashMap::new();
        entry.insert("id".to_string(), Value::String(cid.clone()));
        entry.insert("content".to_string(), Value::Object(content.into_iter().collect()));
        entry.insert("timestamp".to_string(), Value::String(Utc::now().to_rfc3339()));

        self.memory.add(entry.clone());

        let content_hashmap = match entry.get("content").unwrap() {
            Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => HashMap::new(),
        };

        let enc_artefact = self.committer.commit(&cid, &content_hashmap);

        let edge = Hyperedge {
            eid: format!("memory:{}", cid),
            etype: "EpistemicCommit".to_string(),
            vertices: vec![self.agent_v.vid.clone(), format!("data:{}", cid)],
            properties: enc_artefact,
        };
        self.hg.add_hyperedge(edge);

        cid
    }

    pub fn retrieve(&self, query: &str, k: usize) -> Vec<HashMap<String, Value>> {
        let mut results = self.memory.retrieve_relevant(query);
        results.truncate(k);
        results
    }
}
