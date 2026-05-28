use chrono::Utc;
use log::info;
use serde_json::{json, Value};
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;

use crate::memory::{Hyperedge, HypergraphRegistry, Vertex};

pub struct Orkut20Layer<'a> {
    pub agent_id: String,
    pub hg: &'a mut HypergraphRegistry,
    pub profile: HashMap<String, Value>,
}

impl<'a> Orkut20Layer<'a> {
    pub fn new(agent_id: &str, hg: &'a mut HypergraphRegistry) -> Self {
        let mut profile = HashMap::new();
        profile.insert("display_name".to_string(), json!(format!("Arkhean_{}", &agent_id[..8])));
        profile.insert("description".to_string(), json!("Sovereign social entity"));
        profile.insert("friend_count".to_string(), json!(0));
        profile.insert("community_count".to_string(), json!(0));
        profile.insert("scrap_count".to_string(), json!(0));

        Self {
            agent_id: agent_id.to_string(),
            hg,
            profile,
        }
    }

    pub fn create_profile(&mut self, display_name: &str, description: &str, _interests: Option<Vec<String>>) {
        self.profile.insert("display_name".to_string(), json!(display_name));
        self.profile.insert("description".to_string(), json!(description));
        info!("📝 Profile: {}", display_name);
    }

    pub fn create_community(&mut self, name: &str, description: &str, visibility: &str) -> String {
        let mut hasher = Sha3_256::new();
        hasher.update(format!("{}:{}", name, self.agent_id).as_bytes());
        let comm_id = hex::encode(hasher.finalize())[..16].to_string();

        let mut props = HashMap::new();
        props.insert("name".to_string(), json!(name));
        props.insert("description".to_string(), json!(description));
        props.insert("owner".to_string(), json!(self.agent_id));
        props.insert("visibility".to_string(), json!(visibility));

        let comm_vertex = Vertex {
            vid: format!("orkut_community:{}", comm_id),
            vtype: "OrkutCommunity".to_string(),
            properties: props,
        };
        self.hg.add_vertex(comm_vertex.clone());

        let mut edge_props = HashMap::new();
        edge_props.insert("role".to_string(), json!("owner"));

        let edge = Hyperedge {
            eid: format!("orkut_membership:{}:{}", comm_id, self.agent_id),
            etype: "OrkutMembership".to_string(),
            vertices: vec![format!("agent:{}", self.agent_id), comm_vertex.vid],
            properties: edge_props,
        };
        self.hg.add_hyperedge(edge);

        let count = self.profile.get("community_count").and_then(|v| v.as_u64()).unwrap_or(0);
        self.profile.insert("community_count".to_string(), json!(count + 1));

        comm_id
    }

    pub fn join_community(&mut self, community_id: &str) {
        let vid = format!("orkut_community:{}", community_id);
        if !self.hg.vertices.contains_key(&vid) {
            return;
        }

        let mut edge_props = HashMap::new();
        edge_props.insert("role".to_string(), json!("member"));

        let edge = Hyperedge {
            eid: format!("orkut_membership:{}:{}", community_id, self.agent_id),
            etype: "OrkutMembership".to_string(),
            vertices: vec![format!("agent:{}", self.agent_id), vid],
            properties: edge_props,
        };
        self.hg.add_hyperedge(edge);

        let count = self.profile.get("community_count").and_then(|v| v.as_u64()).unwrap_or(0);
        self.profile.insert("community_count".to_string(), json!(count + 1));
    }

    pub fn send_scrap(&mut self, target_agent_id: &str, message: &str, is_public: bool) {
        let mut hasher = Sha3_256::new();
        hasher.update(format!("{}:{}:{}:{}", self.agent_id, target_agent_id, message, Utc::now().to_rfc3339()).as_bytes());
        let scrap_id = hex::encode(hasher.finalize())[..16].to_string();

        let mut props = HashMap::new();
        props.insert("from".to_string(), json!(self.agent_id));
        props.insert("to".to_string(), json!(target_agent_id));
        if is_public {
            props.insert("message".to_string(), json!(message));
        } else {
            props.insert("message".to_string(), json!("[encrypted]"));
        }
        props.insert("is_public".to_string(), json!(is_public));

        let scrap_vertex = Vertex {
            vid: format!("orkut_scrap:{}", scrap_id),
            vtype: "OrkutScrap".to_string(),
            properties: props,
        };
        self.hg.add_vertex(scrap_vertex.clone());

        let edge = Hyperedge {
            eid: format!("orkut_scrap_rel:{}", scrap_id),
            etype: "OrkutScrapRelation".to_string(),
            vertices: vec![format!("agent:{}", self.agent_id), format!("agent:{}", target_agent_id), scrap_vertex.vid],
            properties: HashMap::new(),
        };
        self.hg.add_hyperedge(edge);

        let count = self.profile.get("scrap_count").and_then(|v| v.as_u64()).unwrap_or(0);
        self.profile.insert("scrap_count".to_string(), json!(count + 1));
    }

    pub fn get_profile(&self, agent_id: Option<&str>) -> HashMap<String, Value> {
        if let Some(aid) = agent_id {
            if let Some(vertex) = self.hg.vertices.get(&format!("agent:{}", aid)) {
                return vertex.properties.clone();
            }
            return HashMap::new();
        }
        self.profile.clone()
    }
}
