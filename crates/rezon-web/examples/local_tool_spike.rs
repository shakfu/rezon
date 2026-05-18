// Spike: local-model tool calling via llama-cpp-2's oaicompat surface.
//
// Goal: confirm that `apply_chat_template_with_tools_oaicompat` +
// grammar-lazy sampling + `ChatParseStateOaicompat` produces OpenAI-shaped
// tool_calls deltas from a real GGUF model. Single-turn, no actual tool
// execution - just elicit a tool call and verify the parse stream.
//
// Run:
//   cargo run --example local_tool_spike -- /path/to/model.gguf
//   cargo run --example local_tool_spike -- /path/to/model.gguf "what is 17 * 23?"
//
// Recommended models (must have tool-aware chat template baked in):
//   - Meta Llama 3.1 / 3.2 / 3.3 Instruct (any quant)
//   - Qwen 2.5 / Qwen 3 Instruct (7B+)
//   - Mistral Nemo Instruct
//   - Hermes 3
//
// What the spike prints:
//   1. The rendered prompt (so you can eyeball the tool template).
//   2. Whether a grammar was generated (None = model template lacks tool
//      support; spike will still run but without constrained sampling).
//   3. Live token pieces as they decode.
//   4. Each OpenAI-shaped JSON delta from the streaming parser.
//   5. A final summary.

use std::env;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, GrammarTriggerType, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

const N_CTX: u32 = 4096;
const MAX_NEW_TOKENS: i32 = 1024;
const N_GPU_LAYERS: u32 = 999;

const SYSTEM_PROMPT: &str = "You are a helpful assistant. \
When a calculation or time lookup is needed, call the appropriate tool. \
Respond directly otherwise.";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let model_path = args
        .get(1)
        .ok_or_else(|| anyhow!("usage: local_tool_spike <model.gguf> [prompt]"))?
        .clone();
    let user_prompt = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "What is 17 * 23? Use the calculator tool.".to_string());

    let tools_json = build_tools_json();

    println!("=== loading backend + model ===");
    let backend = Arc::new(LlamaBackend::init().map_err(|e| anyhow!("backend init: {e}"))?);
    let model_params = LlamaModelParams::default().with_n_gpu_layers(N_GPU_LAYERS);
    let model = LlamaModel::load_from_file(&backend, Path::new(&model_path), &model_params)
        .map_err(|e| anyhow!("load_from_file: {e}"))?;
    println!("loaded {model_path}");

    println!("\n=== applying chat template with tools ===");
    let template = model
        .chat_template(None)
        .map_err(|e| anyhow!("chat_template: {e}"))?;
    let messages = vec![
        LlamaChatMessage::new("system".to_string(), SYSTEM_PROMPT.to_string())
            .map_err(|e| anyhow!("system msg: {e}"))?,
        LlamaChatMessage::new("user".to_string(), user_prompt.clone())
            .map_err(|e| anyhow!("user msg: {e}"))?,
    ];
    let tmpl_result = model
        .apply_chat_template_with_tools_oaicompat(
            &template,
            &messages,
            Some(&tools_json),
            None,
            true, // add generation prompt
        )
        .map_err(|e| anyhow!("apply_chat_template_with_tools_oaicompat: {e}"))?;

    println!("--- rendered prompt ---\n{}\n--- end prompt ---", tmpl_result.prompt);
    println!("chat_format       = {}", tmpl_result.chat_format);
    println!("parse_tool_calls  = {}", tmpl_result.parse_tool_calls);
    println!("grammar           = {}", short_present(&tmpl_result.grammar));
    println!("grammar_lazy      = {}", tmpl_result.grammar_lazy);
    println!("grammar_triggers  = {} entries", tmpl_result.grammar_triggers.len());
    for t in &tmpl_result.grammar_triggers {
        println!("  - {:?}: {:?} (token={:?})", t.trigger_type, t.value, t.token);
    }
    println!("preserved_tokens  = {:?}", tmpl_result.preserved_tokens);
    println!("additional_stops  = {:?}", tmpl_result.additional_stops);
    println!("parser            = {}", short_present(&tmpl_result.parser));
    println!("generation_prompt = {:?}", tmpl_result.generation_prompt);

    if tmpl_result.grammar.is_none() {
        println!(
            "\nWARN: model's chat template did not produce a tool grammar. \
             Generation will be unconstrained. The streaming parser may still \
             extract tool calls if the model emits them in the expected format."
        );
    }

    println!("\n=== decoding prompt ===");
    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(N_CTX));
    let mut ctx = model
        .new_context(&backend, ctx_params)
        .map_err(|e| anyhow!("new_context: {e}"))?;

    let prompt_tokens = model
        .str_to_token(&tmpl_result.prompt, AddBos::Always)
        .map_err(|e| anyhow!("str_to_token: {e}"))?;
    println!("prompt token count = {}", prompt_tokens.len());

    let mut batch = LlamaBatch::new(prompt_tokens.len().max(512), 1);
    let last_idx = prompt_tokens.len() - 1;
    for (i, t) in prompt_tokens.iter().enumerate() {
        batch
            .add(*t, i as i32, &[0], i == last_idx)
            .map_err(|e| anyhow!("batch.add prompt: {e}"))?;
    }
    ctx.decode(&mut batch).map_err(|e| anyhow!("decode prompt: {e}"))?;

    println!("\n=== building sampler ===");
    let sampler = build_sampler(&model, &tmpl_result)?;
    let mut sampler = sampler;

    println!("\n=== generating + streaming parse ===");
    let mut parse_state = tmpl_result
        .streaming_state_oaicompat()
        .map_err(|e| anyhow!("streaming_state_oaicompat: {e}"))?;

    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut full = String::new();
    let mut n_cur = prompt_tokens.len() as i32;
    let mut produced = 0i32;
    let max_new = MAX_NEW_TOKENS.min(N_CTX as i32 - n_cur - 8).max(0);

    let stops: Vec<&str> = tmpl_result.additional_stops.iter().map(String::as_str).collect();
    let mut hit_stop = false;

    while produced < max_new {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            println!("\n[eog token reached]");
            break;
        }

        let bytes = model
            .token_to_piece_bytes(token, 64, false, None)
            .map_err(|e| anyhow!("token_to_piece_bytes: {e}"))?;
        let mut piece = String::with_capacity(bytes.len() + 4);
        let _ = decoder.decode_to_string(&bytes, &mut piece, false);

        if !piece.is_empty() {
            // Print the raw piece so you can see what the model is emitting.
            print!("{piece}");
            use std::io::Write;
            std::io::stdout().flush().ok();

            full.push_str(&piece);

            // Feed the piece into the streaming oai-compat parser. Each
            // call returns zero or more JSON delta strings shaped like
            // async-openai's ChatCompletionMessageToolCallChunk / content
            // delta payloads.
            let deltas = parse_state
                .update(&piece, true)
                .map_err(|e| anyhow!("parse_state.update: {e}"))?;
            for d in deltas {
                println!("\n[delta] {d}");
            }
        }

        // Check additional_stops against the accumulated text.
        if stops.iter().any(|s| !s.is_empty() && full.ends_with(s)) {
            println!("\n[hit additional_stop]");
            hit_stop = true;
            break;
        }

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| anyhow!("batch.add gen: {e}"))?;
        n_cur += 1;
        produced += 1;
        ctx.decode(&mut batch).map_err(|e| anyhow!("decode gen: {e}"))?;
    }

    // Final flush: tell the parser the stream is complete so it can emit
    // any buffered terminal deltas.
    let final_deltas = parse_state
        .update("", false)
        .map_err(|e| anyhow!("parse_state.update final: {e}"))?;
    for d in final_deltas {
        println!("\n[delta-final] {d}");
    }

    println!("\n\n=== summary ===");
    println!("generated tokens: {produced}");
    println!("hit_stop:         {hit_stop}");
    println!("raw text length:  {} chars", full.len());

    // Sanity check via the non-streaming parser too, so we have a
    // ground-truth oai-compat message to compare against the streaming
    // deltas.
    match tmpl_result.parse_response_oaicompat(&full, false) {
        Ok(json) => println!("\n=== non-streaming parse ===\n{json}"),
        Err(e) => println!("\nnon-streaming parse error: {e}"),
    }

    Ok(())
}

fn build_tools_json() -> String {
    // OpenAI-compatible tools array.
    serde_json::to_string(&serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "calculator",
                "description": "Evaluate an arithmetic expression like \"17 * 23\".",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "expression": { "type": "string" }
                    },
                    "required": ["expression"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "current_time",
                "description": "Return the current local time.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        }
    ]))
    .expect("serialize tools_json")
}

fn build_sampler(
    model: &LlamaModel,
    r: &llama_cpp_2::model::ChatTemplateResult,
) -> Result<LlamaSampler> {
    // Always finish with temperature + dist sampling.
    let tail = vec![LlamaSampler::temp(0.7), LlamaSampler::dist(1234)];

    if std::env::var("NO_GRAMMAR").is_ok() {
        println!("(NO_GRAMMAR set -> chain_simple [temp, dist], skipping grammar)");
        return Ok(LlamaSampler::chain_simple(tail));
    }
    let Some(grammar_str) = r.grammar.as_deref() else {
        println!("(no grammar -> chain_simple [temp, dist])");
        return Ok(LlamaSampler::chain_simple(tail));
    };

    // Partition triggers: words go to trigger_words, single tokens go to
    // trigger_tokens. Pattern triggers are not directly supported by the
    // grammar_lazy sampler API exposed in 0.1.146; for the spike we fall
    // back to using their text value as a word trigger which is "good
    // enough" to demonstrate the path. (For production we'd use the
    // pattern-aware variant.)
    let mut trigger_words: Vec<Vec<u8>> = Vec::new();
    let mut trigger_tokens: Vec<LlamaToken> = Vec::new();
    let mut had_pattern = false;
    for t in &r.grammar_triggers {
        match t.trigger_type {
            GrammarTriggerType::Token => {
                if let Some(tok) = t.token {
                    trigger_tokens.push(tok);
                }
            }
            GrammarTriggerType::Word => {
                trigger_words.push(t.value.as_bytes().to_vec());
            }
            GrammarTriggerType::Pattern | GrammarTriggerType::PatternFull => {
                had_pattern = true;
                trigger_words.push(t.value.as_bytes().to_vec());
            }
        }
    }
    if had_pattern {
        println!(
            "(note: model template specified pattern triggers; spike approximates them as word triggers)"
        );
    }

    if r.grammar_lazy {
        println!(
            "(building chain_simple [grammar_lazy(words={}, tokens={}), temp, dist])",
            trigger_words.len(),
            trigger_tokens.len()
        );
        let lazy = LlamaSampler::grammar_lazy(model, grammar_str, "root", &trigger_words, &trigger_tokens)
            .context("grammar_lazy")?;
        let mut chain = vec![lazy];
        chain.extend(tail);
        Ok(LlamaSampler::chain_simple(chain))
    } else {
        println!("(building chain_simple [grammar, temp, dist])");
        let g = LlamaSampler::grammar(model, grammar_str, "root").context("grammar")?;
        let mut chain = vec![g];
        chain.extend(tail);
        Ok(LlamaSampler::chain_simple(chain))
    }
}

fn short_present(s: &Option<String>) -> String {
    match s {
        Some(v) => format!("Some({} chars)", v.len()),
        None => "None".to_string(),
    }
}
