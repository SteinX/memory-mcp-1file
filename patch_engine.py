import re

with open("src/embedding/engine.rs", "r") as f:
    content = f.read()

# Add import
content = content.replace(
    "use candle_transformers::models::qwen3::{Config as Qwen3Config, Model as Qwen3Model};",
    "use candle_transformers::models::qwen3::{Config as Qwen3Config, Model as Qwen3Model};\nuse candle_transformers::models::gemma2::{Config as Gemma2Config, Model as Gemma2Model};"
)

# Update InnerModel
content = content.replace(
    "#[allow(dead_code)]\n    Gemma, // Placeholder",
    "Gemma(std::sync::Mutex<Gemma2Model>),"
)

# Update from_files
gemma_impl = """EngineBackend::Gemma => {
                let gemma_cfg: Gemma2Config = serde_json::from_slice(&std::fs::read(config_path)?)?;
                let vb_fixed = vb
                    .rename_f(|name: &str| name.strip_prefix("model.").unwrap_or(name).to_string());
                InnerModel::Gemma(std::sync::Mutex::new(Gemma2Model::new(false, &gemma_cfg, vb_fixed)?))
            }"""
content = re.sub(
    r"EngineBackend::Gemma => \{\s+anyhow::bail!\(.*?\);\s+\}",
    gemma_impl,
    content,
    flags=re.DOTALL
)

# Update embed max_len
max_len_impl = """let max_len = match self.inner {
                    InnerModel::Qwen3(_) => MAX_SEQ_LEN_QWEN3,
                    InnerModel::Gemma(_) => 512,
                    _ => MAX_SEQ_LEN_BERT,
                };"""
content = content.replace(
    "let max_len = match self.inner {\n                    InnerModel::Qwen3(_) => MAX_SEQ_LEN_QWEN3,\n                    _ => MAX_SEQ_LEN_BERT,\n                };",
    max_len_impl
)

# Update embed
embed_gemma = """InnerModel::Gemma(model_mutex) => {
                        let input_ids = Tensor::new(vec![token_ids.clone()], &self.device)?;
                        let mut model_mut = model_mutex
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
                        model_mut.clear_kv_cache();
                        let hidden = model_mut.forward(&input_ids, 0)?;

                        let seq_len = hidden.dim(1)?;
                        let embedding = hidden.narrow(1, seq_len - 1, 1)?.squeeze(1)?;

                        let normalized = l2_normalize(&embedding)?;

                        let vec = normalized.squeeze(0)?.to_vec1::<f32>()?;
                        self.apply_mrl(vec)
                    }"""
content = content.replace(
    "_ => unreachable!(),\n                }",
    embed_gemma + "\n                    _ => unreachable!(),\n                }"
)

# Update embed_batch max_len
content = content.replace(
    "let max_len = match self.inner {\n                    InnerModel::Qwen3(_) => MAX_SEQ_LEN_QWEN3,\n                    _ => MAX_SEQ_LEN_BERT,\n                };",
    max_len_impl
)

# Update embed_batch
embed_batch_gemma = """InnerModel::Gemma(model_mutex) => {
                        let mut model_mut = model_mutex
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

                        let mut results = Vec::with_capacity(texts.len());
                        for (ids, &actual_len) in
                            unpadded_token_ids.iter().zip(actual_lengths.iter())
                        {
                            model_mut.clear_kv_cache();
                            let input = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
                            let hidden = model_mut.forward(&input, 0)?;

                            if actual_len == 0 {
                                return Err(anyhow::anyhow!("Cannot embed empty token sequence"));
                            }
                            let embedding = hidden.narrow(1, actual_len - 1, 1)?.squeeze(1)?;
                            let normalized = l2_normalize(&embedding)?;
                            let vec = normalized.squeeze(0)?.to_vec1::<f32>()?;
                            results.push(self.apply_mrl(vec)?);
                        }
                        Ok(results)
                    }"""
content = content.replace(
    "_ => unreachable!(),\n                }\n            }\n        }\n    }",
    embed_batch_gemma + "\n                    _ => unreachable!(),\n                }\n            }\n        }\n    }"
)

with open("src/embedding/engine.rs", "w") as f:
    f.write(content)
