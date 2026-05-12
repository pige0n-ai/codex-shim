use serde::{Deserialize, Serialize};

use crate::provider_caps::ProviderCapabilities;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub slug: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_level: Option<String>,
    pub supported_reasoning_levels: Vec<ReasoningEffortPreset>,
    pub context_window: i64,
    pub max_context_window: i64,
    pub effective_context_window_percent: i64,
    pub shell_type: String,
    pub visibility: String,
    pub supported_in_api: bool,
    pub priority: i32,
    pub supports_parallel_tool_calls: bool,
    pub input_modalities: Vec<String>,
    pub default_reasoning_summary: String,
    pub supports_reasoning_summaries: bool,
    pub support_verbosity: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_patch_tool_type: Option<String>,
    pub truncation_policy: TruncationPolicyConfig,
    pub supports_image_detail_original: bool,
    pub supports_search_tool: bool,
    pub experimental_supported_tools: Vec<String>,
    pub additional_speed_tiers: Vec<String>,
    pub base_instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<i64>,
    #[serde(default = "default_web_search_tool_type")]
    pub web_search_tool_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningEffortPreset {
    pub effort: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TruncationPolicyConfig {
    pub mode: String,
    pub limit: i64,
}

/// Parse a context window value that may use unit suffixes.
///
/// - lowercase k/m/g = decimal (1_000 / 1_000_000 / 1_000_000_000)
/// - uppercase K/M/G = binary (1_024 / 1_048_576 / 1_073_741_824)
/// - bare integer passes through unchanged
///
/// Examples: `"128K"` → 131072, `"1m"` → 1_000_000, `"131072"` → 131072
pub fn parse_context_window(input: &str) -> Result<i64, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("context_window cannot be empty".into());
    }

    let (num_part, multiplier): (&str, i64) = if let Some(rest) = s.strip_suffix('K') {
        (rest, 1_024)
    } else if let Some(rest) = s.strip_suffix('k') {
        (rest, 1_000)
    } else if let Some(rest) = s.strip_suffix('M') {
        (rest, 1_048_576)
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, 1_000_000)
    } else if let Some(rest) = s.strip_suffix('G') {
        (rest, 1_073_741_824)
    } else if let Some(rest) = s.strip_suffix('g') {
        (rest, 1_000_000_000)
    } else {
        (s, 1)
    };

    let base: f64 = num_part
        .parse()
        .map_err(|_| format!("invalid number: {}", num_part))?;
    let result = (base * multiplier as f64) as i64;
    if result <= 0 {
        return Err(format!("context_window must be positive, got {}", result));
    }
    Ok(result)
}

fn deserialize_context_window<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct CtxWindowVisitor;

    impl<'de> Visitor<'de> for CtxWindowVisitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an integer or a string with optional k/K/m/M/g/G suffix")
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v as i64)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_context_window(v).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(CtxWindowVisitor)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogModelSpec {
    pub slug: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(deserialize_with = "deserialize_context_window")]
    pub context_window: i64,
    #[serde(default)]
    pub tool_calling: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
    #[serde(default)]
    pub reasoning_levels: Option<Vec<String>>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub base_instructions: Option<String>,
    #[serde(default)]
    pub auto_compact_token_limit: Option<i64>,
    #[serde(default)]
    pub supports_search_tool: Option<bool>,
    #[serde(default)]
    pub supports_reasoning_summaries: Option<bool>,
    #[serde(default)]
    pub apply_patch_tool_type: Option<String>,
    #[serde(default)]
    pub supports_image_detail_original: Option<bool>,
}

pub fn build_model_catalog(
    specs: &[CatalogModelSpec],
    caps: &ProviderCapabilities,
) -> ModelsResponse {
    ModelsResponse {
        models: specs
            .iter()
            .map(|spec| build_model_info(spec, caps))
            .collect(),
    }
}

pub fn build_model_info(spec: &CatalogModelSpec, caps: &ProviderCapabilities) -> ModelInfo {
    let reasoning_levels = spec.reasoning_levels.clone().unwrap_or_else(|| {
        if caps.supports_reasoning_effort {
            vec!["high".to_string()]
        } else {
            Vec::new()
        }
    });
    let default_reasoning_level = reasoning_levels.first().cloned();
    let tool_calling = spec.tool_calling.unwrap_or(caps.supports_function_tools);
    let vision = spec.vision.unwrap_or(caps.supports_vision_input);

    let mut input_modalities = vec!["text".to_string()];
    if vision {
        input_modalities.push("image".to_string());
    }

    ModelInfo {
        slug: spec.slug.clone(),
        display_name: spec
            .display_name
            .clone()
            .unwrap_or_else(|| spec.slug.clone()),
        description: spec
            .description
            .clone()
            .or_else(|| Some(format!("{} via codex-shim", spec.slug))),
        default_reasoning_level,
        supported_reasoning_levels: reasoning_levels
            .into_iter()
            .map(|effort| ReasoningEffortPreset {
                effort,
                description: String::new(),
            })
            .collect(),
        context_window: spec.context_window,
        max_context_window: spec.context_window,
        effective_context_window_percent: 95,
        shell_type: if tool_calling {
            "unified_exec".into()
        } else {
            "disabled".into()
        },
        visibility: "list".into(),
        supported_in_api: true,
        priority: spec.priority.unwrap_or(10),
        supports_parallel_tool_calls: caps.supports_parallel_tool_calls,
        input_modalities,
        default_reasoning_summary: "none".into(),
        supports_reasoning_summaries: spec.supports_reasoning_summaries.unwrap_or(false),
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: spec.apply_patch_tool_type.clone(),
        truncation_policy: TruncationPolicyConfig {
            mode: "tokens".into(),
            limit: 10_000,
        },
        supports_image_detail_original: spec.supports_image_detail_original.unwrap_or(false),
        supports_search_tool: spec.supports_search_tool.unwrap_or(false),
        experimental_supported_tools: Vec::new(),
        additional_speed_tiers: Vec::new(),
        base_instructions: spec.base_instructions.clone().unwrap_or_default(),
        auto_compact_token_limit: spec.auto_compact_token_limit,
        web_search_tool_type: default_web_search_tool_type(),
    }
}

fn default_web_search_tool_type() -> String {
    "text".into()
}

#[cfg(test)]
mod context_window_tests {
    use super::parse_context_window;

    #[test]
    fn plain_integer() {
        assert_eq!(parse_context_window("131072").unwrap(), 131072);
    }

    #[test]
    fn lowercase_k_is_1000() {
        assert_eq!(parse_context_window("1k").unwrap(), 1_000);
        assert_eq!(parse_context_window("131k").unwrap(), 131_000);
    }

    #[test]
    fn uppercase_k_is_1024() {
        assert_eq!(parse_context_window("128K").unwrap(), 131072);
        assert_eq!(parse_context_window("1K").unwrap(), 1024);
    }

    #[test]
    fn lowercase_m_is_million() {
        assert_eq!(parse_context_window("1m").unwrap(), 1_000_000);
    }

    #[test]
    fn uppercase_m_is_mebibyte() {
        assert_eq!(parse_context_window("1M").unwrap(), 1_048_576);
    }

    #[test]
    fn lowercase_g_is_billion() {
        assert_eq!(parse_context_window("1g").unwrap(), 1_000_000_000);
    }

    #[test]
    fn uppercase_g_is_gibibyte() {
        assert_eq!(parse_context_window("1G").unwrap(), 1_073_741_824);
    }

    #[test]
    fn fractional_multiplier() {
        assert_eq!(parse_context_window("1.5K").unwrap(), 1536);
        assert_eq!(parse_context_window("0.5m").unwrap(), 500_000);
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_context_window("").is_err());
    }

    #[test]
    fn rejects_invalid() {
        assert!(parse_context_window("abc").is_err());
        assert!(parse_context_window("abcK").is_err());
    }

    #[test]
    fn deserializes_string_suffix_in_catalog() {
        let json = r#"{"slug":"test-model","context_window":"128K"}"#;
        let spec: super::CatalogModelSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.context_window, 131072);
    }

    #[test]
    fn deserializes_plain_integer_in_catalog() {
        let json = r#"{"slug":"test-model","context_window":131072}"#;
        let spec: super::CatalogModelSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.context_window, 131072);
    }
}
