use chrono::Utc;
use log::info;
use serde_json::{json, Value};
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;

use candle_core::Device;
use candle_nn::VarBuilder;

use crate::crypto::OctraService;
use crate::memory::{EncryptedMemoryCommit, EpistemicCommitProtocol, Hyperedge, HypergraphRegistry, MemorySpace, Vertex};
use crate::models::ArkheWorldModel;
use crate::protocol257::Protocol257;
use crate::social::Orkut20Layer;
use crate::web::GoogleGroundingLayer;

#[derive(Clone, Default)]
pub struct ArkheConfig {
    pub maturity: String,
    pub memory_policy: String,
    pub fhe_key_id: String,
    pub zk_domain: String,
    pub pqc_entity_id: String,
    pub registry_endpoint: String,
    pub qpow_enabled: bool,
    pub qpow_backend: String,
    pub google_api_key: Option<String>,
    pub google_cx: Option<String>,
    pub serpapi_key: Option<String>,
    pub google_default_engine: String,
    pub google_auto_ground: bool,
    pub google_max_results: usize,
    pub orkut_enabled: bool,
    pub protocol257_enabled: bool,
}

pub struct ArkheAgent<'a> {
    pub config: ArkheConfig,
    pub agent_id: String,
    pub world_model: ArkheWorldModel,
    pub octra: OctraService,
    pub hypergraph: HypergraphRegistry,
    pub agent_vertex: Vertex,
    pub memory_space: MemorySpace,
    pub epistemic_protocol: Option<EpistemicCommitProtocol<'a>>, // Lifetime management is tricky in a struct, we'll implement inline for simplicity or just keep it simpler in rust
    pub google: Option<GoogleGroundingLayer>,
    pub orkut: Option<Orkut20Layer<'a>>,
    pub protocol257: Option<Protocol257>,
    pub total_commits: u32,
    pub total_interactions: u32,
    pub total_web_queries: u32,
    pub device: Device,
}

impl<'a> ArkheAgent<'a> {
    pub fn new(config: ArkheConfig) -> candle_core::Result<Self> {
        let mut hasher = Sha3_256::new();
        hasher.update(format!("ARKHE-AGENT-{}", Utc::now().to_rfc3339()).as_bytes());
        let agent_id = hex::encode(hasher.finalize())[..16].to_string();
        info!("🤖 Arkhe Agent {} initialising…", agent_id);

        let device = Device::Cpu; // Fallback to CPU for mock
        let vb = VarBuilder::zeros(candle_core::DType::F32, &device);

        let world_model = ArkheWorldModel::new(256, 64, &config.maturity, vb)?;

        let mut octra = OctraService::new();
        octra.provision_fhe(&config.fhe_key_id, 3);
        octra.provision_zk(&config.zk_domain, 2, 3);
        octra.provision_pqc(&config.pqc_entity_id, 3);

        let mut hypergraph = HypergraphRegistry::new(&config.registry_endpoint);

        let mut props = HashMap::new();
        props.insert("maturity".to_string(), json!(config.maturity));
        props.insert("timestamp".to_string(), json!(Utc::now().to_rfc3339()));

        let agent_vertex = Vertex {
            vid: format!("agent:{}", agent_id),
            vtype: "AGI_Agent".to_string(),
            properties: props,
        };
        hypergraph.add_vertex(agent_vertex.clone());

        let memory_space = MemorySpace::new(&agent_id);

        let google = if config.google_auto_ground {
            Some(GoogleGroundingLayer::new(
                config.google_api_key.clone(),
                config.google_cx.clone(),
                config.serpapi_key.clone(),
                &config.google_default_engine,
            ))
        } else {
            None
        };

        let mut protocol257 = None;
        if config.protocol257_enabled {
            let mut p257 = Protocol257::new(&agent_id);
            p257.set_shared_seed("vitral da catedral oculta", "");
            p257.start_session();
            protocol257 = Some(p257);
        }

        info!("✅ Arkhe Agent ready — Trinitarian + Google + Orkut + Proto257 active.");

        Ok(Self {
            config,
            agent_id,
            world_model,
            octra,
            hypergraph,
            agent_vertex,
            memory_space,
            epistemic_protocol: None, // Will use memory manually in the port to avoid complex self-referential lifetimes
            google,
            orkut: None, // Will instantiate later when needed
            protocol257,
            total_commits: 0,
            total_interactions: 0,
            total_web_queries: 0,
            device,
        })
    }

        pub fn perceive(&mut self, text_input: &str, peptide_seq: Option<&str>, web_query: Option<&str>) -> candle_core::Result<Value> {
        self.total_interactions += 1;

        let mut synthesized_context = "".to_string();
        let mut web_grounded = false;

        let mut web_ctx_tensor: Option<candle_core::Tensor> = None;

        if let Some(google) = &mut self.google {
            if self.config.google_auto_ground || web_query.is_some() {
                let query = web_query.unwrap_or(text_input);
                let search_results = google.search(query, None, self.config.google_max_results);
                self.total_web_queries += 1;
                synthesized_context = google.synthesize_context(&search_results, 3);
                web_grounded = true;

                // mock web context embedding
                web_ctx_tensor = Some(candle_core::Tensor::zeros((1, 512), candle_core::DType::F32, &self.device)?);
            }
        }

        // mock tokens
        let tokens = candle_core::Tensor::zeros((1, 10, 256), candle_core::DType::F32, &self.device)?;
        let action = candle_core::Tensor::zeros((1, 64), candle_core::DType::F32, &self.device)?;

        let (_state, _causal_effect, conf, uncert, nov) = self.world_model.forward(&tokens, &action, peptide_seq, web_ctx_tensor.as_ref(), &self.device)?;

        let perception = json!({
            "timestamp": Utc::now().to_rfc3339(),
            "input_text": text_input.chars().take(200).collect::<String>().as_str(),
            "web_grounded": web_grounded,
            "web_context": synthesized_context.chars().take(500).collect::<String>().as_str(),
            "self_model": {
                "confidence": conf,
                "uncertainty": uncert,
                "novelty": nov,
            }
        });

        Ok(perception)
    }
    pub fn reason(&mut self, perception: &Value) -> Value {
        let input_text = perception["input_text"].as_str().unwrap_or("");
        let relevant = self.memory_space.retrieve_relevant(input_text);

        let web_grounded = perception["web_grounded"].as_bool().unwrap_or(false);
        let web_boost = if web_grounded { 0.1 } else { 0.0 };
        let mut confidence = perception["self_model"]["confidence"].as_f64().unwrap_or(0.9) + web_boost;
        if confidence > 0.95 { confidence = 0.95; }

        json!({
            "type": "respond",
            "confidence": confidence,
            "based_on_memories": relevant.len(),
            "web_grounded": web_grounded,
        })
    }

    pub fn act(&self, action: &Value) -> String {
        if action["type"].as_str().unwrap_or("") == "respond" {
            let web_tag = if action["web_grounded"].as_bool().unwrap_or(false) { "[WEB-GROUNDED] " } else { "" };
            let conf = action["confidence"].as_f64().unwrap_or(0.0);
            format!("{}Agent {} acting with confidence {:.2}", web_tag, self.agent_id, conf)
        } else {
            "No action taken.".to_string()
        }
    }

    pub fn commit_memory(&mut self, content: HashMap<String, Value>) -> String {
        // Implement EpistemicCommitProtocol logic directly to avoid lifetime borrowing issues
        let content_str = serde_json::to_string(&content).unwrap_or_default();
        let mut hasher = Sha3_256::new();
        hasher.update(content_str.as_bytes());
        let cid = hex::encode(hasher.finalize())[..16].to_string();

        let mut entry = HashMap::new();
        entry.insert("id".to_string(), json!(cid));
        entry.insert("content".to_string(), json!(content));
        entry.insert("timestamp".to_string(), json!(Utc::now().to_rfc3339()));

        self.memory_space.add(entry);

        let mut committer = EncryptedMemoryCommit::new(
            &mut self.octra,
            &self.agent_id,
            &self.config.fhe_key_id,
            &self.config.zk_domain,
            &self.config.pqc_entity_id,
        );

        let enc_artefact = committer.commit(&cid, &content);

        let edge = Hyperedge {
            eid: format!("memory:{}", cid),
            etype: "EpistemicCommit".to_string(),
            vertices: vec![self.agent_vertex.vid.clone(), format!("data:{}", cid)],
            properties: enc_artefact,
        };
        self.hypergraph.add_hyperedge(edge);

        self.total_commits += 1;
        info!("💾 Memory commit {}… sealed.", &cid[..12]);
        cid
    }

    pub fn report(&self) -> String {
        let proto257_status = if let Some(p) = &self.protocol257 {
            if p.current_session_nonce.is_some() { "active" } else { "inactive" }
        } else { "inactive" };

        format!(r#"
╔══════════════════════════════════════════════════════════╗
║ ARKHE AGENT REPORT – {} ║
╠══════════════════════════════════════════════════════════╣
║ Interactions: {:>33}
║ Explicit Commits: {:>33}
║ Memory Policy: {:>33}
║ qPoW Enabled: {:>33}
║ World-Model: {:>33}
║ Protocol 257 session: {:>33}
║ Orkut 2.0: {:>33}
╚══════════════════════════════════════════════════════════╝"#,
            self.agent_id,
            self.total_interactions,
            self.total_commits,
            self.config.memory_policy,
            self.config.qpow_enabled,
            self.config.maturity,
            proto257_status,
            if self.config.orkut_enabled { "active" } else { "inactive" }
        )
    }
}
