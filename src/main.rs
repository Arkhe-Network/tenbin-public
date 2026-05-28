mod agent;
mod crypto;
mod memory;
mod models;
mod permaweb;
mod protocol257;
mod social;
mod web;

use clap::Parser;
use log::info;
use serde_json::json;
use std::collections::HashMap;

use crate::agent::{ArkheAgent, ArkheConfig};
use crate::social::Orkut20Layer;

#[derive(Parser, Debug)]
#[command(author, version, about = "Arkhe-OS.gguf Complete AGI", long_about = None)]
struct Args {
    #[arg(long, default_value = "infant", value_parser = ["embryo", "infant", "adult"])]
    maturity: String,

    #[arg(long)]
    qpow: bool,

    #[arg(long, default_value = "")]
    google_key: String,

    #[arg(long, default_value = "")]
    google_cx: String,

    #[arg(long, default_value = "")]
    serpapi_key: String,

    #[arg(long, default_value = "google")]
    engine: String,

    #[arg(long)]
    no_web: bool,

    #[arg(long)]
    no_orkut: bool,

    #[arg(long)]
    no_proto257: bool,

    #[arg(long)]
    no_permaweb: bool,
}

fn main() -> candle_core::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let config = ArkheConfig {
        maturity: args.maturity,
        memory_policy: "encrypted".to_string(),
        fhe_key_id: "arkhe-agent-001".to_string(),
        zk_domain: "arkhe.epistemic".to_string(),
        pqc_entity_id: "arkhe-agent-001-pqc".to_string(),
        registry_endpoint: "localhost:8720".to_string(),
        qpow_enabled: args.qpow,
        qpow_backend: "qasm_simulator".to_string(),
        google_api_key: if args.google_key.is_empty() { None } else { Some(args.google_key) },
        google_cx: if args.google_cx.is_empty() { None } else { Some(args.google_cx) },
        serpapi_key: if args.serpapi_key.is_empty() { None } else { Some(args.serpapi_key) },
        google_default_engine: args.engine,
        google_auto_ground: !args.no_web,
        google_max_results: 3,
        orkut_enabled: !args.no_orkut,
        protocol257_enabled: !args.no_proto257,
        permaweb_enabled: !args.no_permaweb,
    };

    let mut agent = ArkheAgent::new(config.clone())?;
    println!("{}", agent.report());

    if let Some(p257) = &agent.protocol257 {
        println!("\n🔒 Protocolo 257 — Linguagem Sem Raiz");
        let plain = "eu preciso de água depressa";
        let enc = p257.encode_message(plain);
        println!("Plain:  {}", plain);
        println!("Encoded: {}", enc);
        let dec = p257.decode_message(&enc);
        println!("Decoded: {}", dec);

        let carrier = "O tempo está bom hoje, mas pode chover mais tarde.";
        let stego = p257.steganographic_embed(&enc, carrier);
        println!("\nStego carrier: {}", stego);
        let extracted = p257.steganographic_extract(&stego);
        println!("Extracted message: {}", extracted);
    }

    if config.orkut_enabled {
        let mut orkut = Orkut20Layer::new(&agent.agent_id, &mut agent.hypergraph);
        orkut.create_profile("Cidadão da Catedral", "Soberano digital", None);
        println!("Orkut profile created.");

        // We can simulate an interaction where the agent stores something in memory
        let mut mem_content = HashMap::new();
        mem_content.insert("type".to_string(), json!("orkut_profile"));
        mem_content.insert("display_name".to_string(), json!("Cidadão da Catedral"));
        agent.commit_memory(mem_content);
    }

    println!("\n⚡ Arkhe‑OS.gguf completo está vivo.");

    Ok(())
}
