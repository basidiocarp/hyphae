use anyhow::Result;
use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

#[derive(Debug, Clone, Copy)]
pub enum TrainingFormat {
    Sft,
    Dpo,
    Alpaca,
}

impl std::str::FromStr for TrainingFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sft" => Ok(TrainingFormat::Sft),
            "dpo" => Ok(TrainingFormat::Dpo),
            "alpaca" => Ok(TrainingFormat::Alpaca),
            _ => Err(format!("unknown format: {s}")),
        }
    }
}

impl std::fmt::Display for TrainingFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrainingFormat::Sft => write!(f, "sft"),
            TrainingFormat::Dpo => write!(f, "dpo"),
            TrainingFormat::Alpaca => write!(f, "alpaca"),
        }
    }
}

#[derive(serde::Serialize)]
struct SftRecord {
    instruction: String,
    response: String,
}

#[derive(serde::Serialize)]
struct DpoRecord {
    prompt: String,
    chosen: String,
    rejected: String,
}

#[derive(serde::Serialize)]
struct AlpacaRecord {
    instruction: String,
    input: String,
    output: String,
}

/// ─────────────────────────────────────────────────────────────────────────
/// Export Memories as Training Data
/// ─────────────────────────────────────────────────────────────────────────
pub(crate) fn cmd_export_training(
    store: &SqliteStore,
    format: TrainingFormat,
    topic: Option<String>,
    min_weight: Option<f32>,
    project: Option<String>,
) -> Result<()> {
    let min_weight = min_weight.unwrap_or(0.0);
    let project_ref = project.as_deref();

    // Determine topics to export
    let topics = if let Some(t) = topic {
        vec![t]
    } else {
        // Default topics for training data
        vec![
            "decisions".to_string(),
            "errors/resolved".to_string(),
            "context".to_string(),
            "session".to_string(),
            "corrections".to_string(),
        ]
    };

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    use std::io::Write;

    for t in topics {
        match store.get_by_topic(&t, project_ref) {
            Ok(memories) => {
                for mem in memories {
                    if mem.weight.value() < min_weight {
                        continue;
                    }

                    match format {
                        TrainingFormat::Sft => {
                            let instruction = format!("What is our convention for: {}", mem.topic);
                            let response = mem.summary.clone();
                            let record = SftRecord {
                                instruction,
                                response,
                            };
                            writeln!(
                                handle,
                                "{}",
                                serde_json::to_string(&record)?
                            )?;
                        }
                        TrainingFormat::Dpo => {
                            // Only export DPO if it looks like a correction
                            if mem.topic == "corrections" || mem.summary.contains("Original:")
                            {
                                if let Some((rejected, chosen)) =
                                    parse_correction(&mem.summary)
                                {
                                    let prompt = format!("Fix the code: {}", mem.topic);
                                    let record = DpoRecord {
                                        prompt,
                                        chosen,
                                        rejected,
                                    };
                                    writeln!(
                                        handle,
                                        "{}",
                                        serde_json::to_string(&record)?
                                    )?;
                                }
                            }
                        }
                        TrainingFormat::Alpaca => {
                            let instruction = format!("Topic: {}", mem.topic);
                            let output = mem.summary.clone();
                            let record = AlpacaRecord {
                                instruction,
                                input: String::new(),
                                output,
                            };
                            writeln!(
                                handle,
                                "{}",
                                serde_json::to_string(&record)?
                            )?;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("failed to read topic {}: {}", t, e);
            }
        }
    }

    Ok(())
}

/// ─────────────────────────────────────────────────────────────────────────
/// Parse Corrections
/// ─────────────────────────────────────────────────────────────────────────
fn parse_correction(text: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();

    let mut original: Option<String> = None;
    let mut correction: Option<String> = None;

    for line in lines {
        if line.starts_with("Original:") {
            original = Some(
                line.strip_prefix("Original:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        } else if line.starts_with("Correction:") {
            correction = Some(
                line.strip_prefix("Correction:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        }
    }

    match (original, correction) {
        (Some(o), Some(c)) if !o.is_empty() && !c.is_empty() => Some((o, c)),
        _ => None,
    }
}
