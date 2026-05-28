use chrono::Utc;
use log::info;
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;

pub struct Protocol257 {
    pub agent_id: String,
    pub shared_seed: Option<Vec<u8>>,
    pub current_session_nonce: Option<Vec<u8>>,
    pub vocabulary: HashMap<String, String>,
    pub reverse_vocab: HashMap<String, String>,
    pub grammar_rules: HashMap<String, String>,
}

impl Protocol257 {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            shared_seed: None,
            current_session_nonce: None,
            vocabulary: HashMap::new(),
            reverse_vocab: HashMap::new(),
            grammar_rules: HashMap::new(),
        }
    }

    pub fn set_shared_seed(&mut self, description: &str, salt: &str) {
        let mut hasher = Sha3_256::new();
        hasher.update(description.as_bytes());
        hasher.update(salt.as_bytes());
        self.shared_seed = Some(hasher.finalize().to_vec());
    }

    pub fn start_session(&mut self) {
        if self.shared_seed.is_none() {
            panic!("Shared seed must be set first.");
        }

        let seed_hex = hex::encode(self.shared_seed.as_ref().unwrap());
        let raw = format!("{}:{}:{}", seed_hex, self.agent_id, Utc::now().to_rfc3339());

        let mut hasher = Sha3_256::new();
        hasher.update(raw.as_bytes());
        self.current_session_nonce = Some(hasher.finalize().to_vec());

        self.generate_vocabulary();
        self.generate_grammar();
        info!("📜 Session started with ephemeral vocab ({} words)", self.vocabulary.len());
    }

    fn generate_vocabulary(&mut self) {
        let base_words = vec![
            "eu", "tu", "ele", "nós", "vós", "eles", "sim", "não", "comida", "água", "casa", "perigo", "seguro",
            "ir", "vir", "ver", "ouvir", "dizer", "pensar", "sentir", "bom", "mau", "rápido", "lento", "grande", "pequeno",
        ];

        let seed_hex = hex::encode(self.shared_seed.as_ref().unwrap());
        let nonce_hex = hex::encode(self.current_session_nonce.as_ref().unwrap());

        self.vocabulary.clear();
        self.reverse_vocab.clear();

        for w in base_words {
            let raw = format!("{}:{}:{}", w, seed_hex, nonce_hex);
            let mut hasher = Sha3_256::new();
            hasher.update(raw.as_bytes());
            let hash = hex::encode(hasher.finalize());
            let gen_word = &hash[..5];

            self.vocabulary.insert(w.to_string(), gen_word.to_string());
            self.reverse_vocab.insert(gen_word.to_string(), w.to_string());
        }
    }

    fn generate_grammar(&mut self) {
        let nonce = self.current_session_nonce.as_ref().unwrap();
        let mut seed = [0u8; 32];
        let bytes_to_copy = std::cmp::min(8, nonce.len() - 8);
        seed[..bytes_to_copy].copy_from_slice(&nonce[8..8 + bytes_to_copy]);
        let mut rng = StdRng::from_seed(seed);

        let orders = ["SVO", "SOV", "OSV", "VSO", "OVS", "VOS"];
        let word_order = orders[rng.gen_range(0..6)].to_string();

        let mut hasher = Sha3_256::new();
        hasher.update(format!("filler{}", hex::encode(nonce)).as_bytes());
        let filler_particle = hex::encode(hasher.finalize())[..3].to_string();

        self.grammar_rules.insert("word_order".to_string(), word_order);
        self.grammar_rules.insert("filler_particle".to_string(), filler_particle);
        self.grammar_rules.insert("compound_delimiter".to_string(), "-".to_string());
    }

    pub fn encode_message(&self, plaintext: &str) -> String {
        if self.vocabulary.is_empty() {
            panic!("No active session. Call start_session() first.");
        }

        let plaintext_lower = plaintext.to_lowercase();
        let plaintext_lower = plaintext.to_lowercase();
        let words: Vec<&str> = plaintext_lower.split_whitespace().collect();
        let mut translated = Vec::new();

        for w in words {
            let core = w.trim_matches(|c| ".,!?;:".contains(c));
            if let Some(v) = self.vocabulary.get(core) {
                translated.push(v.clone());
            } else {
                translated.push(self.compound_unknown(core));
            }
        }

        self.apply_grammar(&translated)
    }

    fn compound_unknown(&self, word: &str) -> String {
        let mut hasher = Sha3_256::new();
        hasher.update(word.as_bytes());
        let hash = hex::encode(hasher.finalize());
        let h = u32::from_str_radix(&hash[..8], 16).unwrap_or(0);

        let known: Vec<&String> = self.vocabulary.values().collect();
        if known.is_empty() {
            return word.to_string();
        }

        let w1 = known[(h as usize) % known.len()];
        let w2 = known[((h as usize) * 7) % known.len()];
        format!("{}{}{}", w1, self.grammar_rules.get("compound_delimiter").unwrap_or(&"-".to_string()), w2)
    }

    fn apply_grammar(&self, words: &[String]) -> String {
        let nonce = self.current_session_nonce.as_ref().unwrap();
        let mut seed = [0u8; 32];
        let bytes_to_copy = std::cmp::min(8, nonce.len() - 16);
        seed[..bytes_to_copy].copy_from_slice(&nonce[16..16 + bytes_to_copy]);
        let mut rng = StdRng::from_seed(seed);

        let mut filled = Vec::new();
        let filler = self.grammar_rules.get("filler_particle").unwrap();

        for w in words {
            filled.push(w.clone());
            if rng.gen::<f32>() < 0.2 {
                filled.push(filler.clone());
            }
        }

        if let Some(order) = self.grammar_rules.get("word_order") {
            if order == "OSV" && filled.len() >= 3 {
                filled.swap(0, 1);
            }
        }

        filled.join(" ")
    }

    pub fn decode_message(&self, encoded: &str) -> String {
        if self.reverse_vocab.is_empty() {
            panic!("No vocabulary loaded.");
        }

        let words: Vec<&str> = encoded.split_whitespace().collect();
        let mut decoded = Vec::new();

        let delim = self.grammar_rules.get("compound_delimiter").unwrap_or(&"-".to_string()).clone();
        let filler = self.grammar_rules.get("filler_particle").unwrap_or(&"".to_string()).clone();

        for w in words {
            if let Some(v) = self.reverse_vocab.get(w) {
                decoded.push(v.clone());
            } else if w.contains(&delim) {
                decoded.push(w.replace(&delim, "_"));
            } else if w == filler {
                continue;
            } else {
                decoded.push(format!("<{}>", w));
            }
        }

        decoded.join(" ")
    }

    pub fn steganographic_embed(&self, secret_msg: &str, carrier_text: &str) -> String {
        let mut binary = String::new();
        for c in secret_msg.chars() {
            binary.push_str(&format!("{:08b}", c as u8));
        }

        let words: Vec<&str> = carrier_text.split_whitespace().collect();
        let mut result = Vec::new();

        for (i, bit) in binary.chars().enumerate() {
            if i >= words.len() {
                break;
            }
            let mut w = words[i].to_string();
            if bit == '1' {
                w = w.chars().next().unwrap().to_uppercase().collect::<String>() + &w[1..];
            } else {
                w = w.chars().next().unwrap().to_lowercase().collect::<String>() + &w[1..];
            }
            result.push(w);
        }

        for i in binary.len()..words.len() {
            result.push(words[i].to_string());
        }

        result.join(" ")
    }

    pub fn steganographic_extract(&self, stego_text: &str) -> String {
        let words: Vec<&str> = stego_text.split_whitespace().collect();
        let mut bits = String::new();

        for w in words {
            if w.is_empty() {
                continue;
            }
            if w.chars().next().unwrap().is_uppercase() {
                bits.push('1');
            } else {
                bits.push('0');
            }
        }

        let mut chars = String::new();
        for i in (0..bits.len()).step_by(8) {
            if i + 8 > bits.len() {
                break;
            }
            if let Ok(byte) = u8::from_str_radix(&bits[i..i + 8], 2) {
                chars.push(byte as char);
            }
        }

        chars
    }
}
