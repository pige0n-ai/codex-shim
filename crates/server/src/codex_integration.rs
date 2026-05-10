use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use protocol::{
    models::{CatalogModelSpec, ModelsResponse, build_model_catalog},
    provider_caps::ProviderCapabilities,
};
use providers::{self, preset_capabilities};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::{config::Config, provider_profile_config::ProviderProfileConfig};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexIntegrationOptions {
    #[serde(default = "default_provider_id")]
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub codex_home: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub trust_project: bool,
    #[serde(default)]
    pub env_key: Option<String>,
    #[serde(default)]
    pub web_search: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub base_toml_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexIntegrationPreview {
    pub mode: String,
    pub provider_id: String,
    pub target_model: String,
    pub target_path: String,
    pub catalog_path: String,
    pub base_url: String,
    pub original_toml: String,
    pub merged_toml: String,
    pub catalog_json: String,
    pub trust_target_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexIntegrationApplyResult {
    pub config_path: Option<String>,
    pub target_path: String,
    pub catalog_path: String,
    pub trust_target_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DesktopCheckStatus {
    Supported,
    Gated,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopCheck {
    pub status: DesktopCheckStatus,
    pub subject: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopDoctorReport {
    pub checks: Vec<DesktopCheck>,
}

impl DesktopDoctorReport {
    pub fn has_unsupported(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == DesktopCheckStatus::Unsupported)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShimConfigSummary {
    pub listen: String,
    pub provider_kind: String,
    pub upstream_base_url: String,
    pub model: String,
    pub state_backend: String,
}

pub fn default_provider_id() -> String {
    "codex_shim".into()
}

pub fn parse_config_text(config_text: &str) -> anyhow::Result<Config> {
    let config: Config = serde_yaml::from_str(config_text)?;
    config.validate()?;
    Ok(config)
}

pub fn load_config_text(path: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

pub fn save_config_text(path: &Path, config_text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, config_text).with_context(|| format!("failed to write {}", path.display()))
}

pub fn config_summary(config: &Config) -> ShimConfigSummary {
    ShimConfigSummary {
        listen: config.server.listen.clone(),
        provider_kind: config.provider.kind.clone(),
        upstream_base_url: config.upstream.base_url.clone(),
        model: config.models.default.clone(),
        state_backend: config.state.backend.clone(),
    }
}

pub fn render_model_catalog_json(
    config: &Config,
    profile_override: Option<&str>,
) -> anyhow::Result<String> {
    let caps = match profile_override {
        Some(profile) => explicit_profile_caps(profile)?,
        None => configured_profile_caps(config),
    };
    let catalog = build_model_catalog(&config.models.catalog, &caps);
    let mut rendered = serde_json::to_string_pretty(&catalog)?;
    rendered.push('\n');
    Ok(rendered)
}

pub fn preview_codex_integration(
    config: &Config,
    options: &CodexIntegrationOptions,
) -> anyhow::Result<CodexIntegrationPreview> {
    let resolved = resolve_target(config, options)?;
    let original_toml = match &options.base_toml_override {
        Some(override_text) => override_text.clone(),
        None => match std::fs::read_to_string(&resolved.target_path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to read {}", resolved.target_path.display()));
            }
        },
    };

    let mut doc = parse_toml_document(original_toml.as_str(), resolved.target_path.as_path())?;
    merge_codex_config_document(
        &mut doc,
        &resolved.provider_id,
        &resolved.target_model,
        &resolved.catalog_path,
        &resolved.base_url,
        resolved.env_key.as_deref(),
        resolved.effective_web_search.as_deref(),
    )?;

    let mut merged_toml = doc.to_string();
    if !merged_toml.ends_with('\n') {
        merged_toml.push('\n');
    }

    let mut catalog_json = serde_json::to_string_pretty(&resolved.catalog)?;
    catalog_json.push('\n');

    Ok(CodexIntegrationPreview {
        mode: resolved.mode,
        provider_id: resolved.provider_id,
        target_model: resolved.target_model,
        target_path: resolved.target_path.display().to_string(),
        catalog_path: resolved.catalog_path.display().to_string(),
        base_url: resolved.base_url,
        original_toml,
        merged_toml,
        catalog_json,
        trust_target_path: resolved
            .trust_target_path
            .map(|path| path.display().to_string()),
    })
}

pub fn apply_codex_integration(
    config_text: &str,
    config_path: Option<&Path>,
    options: &CodexIntegrationOptions,
) -> anyhow::Result<CodexIntegrationApplyResult> {
    let config = parse_config_text(config_text)?;
    let preview = preview_codex_integration(&config, options)?;

    if let Some(path) = config_path {
        save_config_text(path, config_text)?;
    }

    let catalog_path = PathBuf::from(&preview.catalog_path);
    if let Some(parent) = catalog_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&catalog_path, preview.catalog_json.as_bytes())
        .with_context(|| format!("failed to write {}", catalog_path.display()))?;

    let target_path = PathBuf::from(&preview.target_path);
    write_toml_document(&target_path, &preview.merged_toml)?;

    if options.project_dir.is_some() && options.trust_project {
        let codex_home = resolve_codex_home(options.codex_home.as_deref())?;
        std::fs::create_dir_all(&codex_home)
            .with_context(|| format!("failed to create CODEX_HOME at {}", codex_home.display()))?;
        let global_path = codex_home.join("config.toml");
        let project_dir = resolve_project_dir(
            options
                .project_dir
                .as_deref()
                .ok_or_else(|| anyhow!("missing project_dir"))?,
        )?;
        update_project_trust_entry(&global_path, &project_dir)?;
    }

    Ok(CodexIntegrationApplyResult {
        config_path: config_path.map(|path| path.display().to_string()),
        target_path: preview.target_path,
        catalog_path: preview.catalog_path,
        trust_target_path: preview.trust_target_path,
    })
}

pub fn doctor_desktop(
    config: &Config,
    options: &CodexIntegrationOptions,
) -> anyhow::Result<DesktopDoctorReport> {
    let project_dir = resolve_project_dir(
        options
            .project_dir
            .as_deref()
            .ok_or_else(|| anyhow!("doctor_desktop requires project_dir"))?,
    )?;
    let provider_id = if options.provider_id.trim().is_empty() {
        default_provider_id()
    } else {
        options.provider_id.clone()
    };
    build_desktop_doctor_report(
        config,
        &configured_caps_with_override(config, options.profile.as_deref())?,
        &project_dir,
        options.codex_home.as_deref(),
        &provider_id,
    )
}

struct ResolvedIntegration {
    mode: String,
    provider_id: String,
    target_model: String,
    target_path: PathBuf,
    catalog_path: PathBuf,
    base_url: String,
    env_key: Option<String>,
    effective_web_search: Option<String>,
    catalog: ModelsResponse,
    trust_target_path: Option<PathBuf>,
}

fn resolve_target(
    config: &Config,
    options: &CodexIntegrationOptions,
) -> anyhow::Result<ResolvedIntegration> {
    let provider_id = if options.provider_id.trim().is_empty() {
        default_provider_id()
    } else {
        options.provider_id.clone()
    };
    let target_model = options
        .model
        .clone()
        .unwrap_or_else(|| config.models.default.clone());
    let available_models: Vec<&str> = config
        .models
        .catalog
        .iter()
        .map(|spec| spec.slug.as_str())
        .collect();
    if !available_models.contains(&target_model.as_str()) {
        bail!(
            "model '{}' is not present in shim models.catalog. Available models: {}",
            target_model,
            available_models.join(", ")
        );
    }

    let caps = configured_caps_with_override(config, options.profile.as_deref())?;
    let catalog = build_model_catalog(&config.models.catalog, &caps);
    let effective_web_search = resolve_install_web_search_mode(
        options.web_search.as_deref(),
        &caps,
        &config.models.catalog,
    )?
    .map(ToOwned::to_owned);

    let base_url = options
        .base_url
        .clone()
        .unwrap_or_else(|| format!("http://{}{}", config.server.listen, config.server.base_path));

    if let Some(project_dir) = options.project_dir.as_deref() {
        if provider_id != default_provider_id() {
            bail!(
                "desktop project installs require provider_id '{}' to keep resume/history stable",
                default_provider_id()
            );
        }
        let project_dir = resolve_project_dir(project_dir)?;
        let trust_target_path = if options.trust_project {
            Some(resolve_codex_home(options.codex_home.as_deref())?.join("config.toml"))
        } else {
            None
        };
        Ok(ResolvedIntegration {
            mode: "project".into(),
            provider_id,
            target_model,
            target_path: project_config_path(&project_dir),
            catalog_path: project_catalog_path(&project_dir)?,
            base_url,
            env_key: options.env_key.clone(),
            effective_web_search,
            catalog,
            trust_target_path,
        })
    } else {
        let codex_home = resolve_codex_home(options.codex_home.as_deref())?;
        Ok(ResolvedIntegration {
            mode: "global".into(),
            provider_id,
            target_model,
            target_path: codex_home.join("config.toml"),
            catalog_path: codex_home.join("model-catalog-shim.json"),
            base_url,
            env_key: options.env_key.clone(),
            effective_web_search,
            catalog,
            trust_target_path: None,
        })
    }
}

fn configured_caps_with_override(
    config: &Config,
    profile_override: Option<&str>,
) -> anyhow::Result<ProviderCapabilities> {
    Ok(match profile_override {
        Some(profile) => explicit_profile_caps(profile)?,
        None => configured_profile_caps(config),
    })
}

fn configured_profile_caps(config: &Config) -> ProviderCapabilities {
    let profile_cfg =
        config
            .provider
            .profile_config
            .clone()
            .unwrap_or_else(|| ProviderProfileConfig {
                profile: config.provider.kind.clone(),
                ..Default::default()
            });
    profile_cfg.build_profile().capabilities().clone()
}

fn explicit_profile_caps(profile: &str) -> anyhow::Result<ProviderCapabilities> {
    if !providers::is_supported_profile_name(profile) {
        bail!(
            "unknown provider profile '{}'. Supported profiles: {}",
            profile,
            providers::SUPPORTED_PROFILE_NAMES.join(", ")
        );
    }
    Ok(preset_capabilities(profile)
        .unwrap_or_else(protocol::provider_caps::ProviderCapabilities::generic_chat))
}

fn resolve_install_web_search_mode<'a>(
    requested: Option<&'a str>,
    caps: &ProviderCapabilities,
    catalog: &[CatalogModelSpec],
) -> anyhow::Result<Option<&'a str>> {
    match requested {
        Some(mode @ ("disabled" | "cached" | "live")) => {
            if mode != "disabled" && !catalog_supports_web_search(caps, catalog) {
                bail!(
                    "web_search mode '{}' requires a Responses-capable profile with hosted web search enabled and every advertised catalog model to set supports_search_tool = true",
                    mode
                );
            }
            Ok(Some(mode))
        }
        Some(other) => bail!(
            "invalid web_search mode '{}'; expected one of: disabled, cached, live",
            other
        ),
        None if catalog_supports_web_search(caps, catalog) => Ok(None),
        None => Ok(Some("disabled")),
    }
}

fn catalog_supports_web_search(caps: &ProviderCapabilities, catalog: &[CatalogModelSpec]) -> bool {
    caps.supports_hosted_web_search
        && !catalog.is_empty()
        && catalog
            .iter()
            .all(|spec| spec.supports_search_tool.unwrap_or(false))
}

fn resolve_project_dir(project_dir: &str) -> anyhow::Result<PathBuf> {
    let path = absolutize(&crate::config::expand_tilde(project_dir))?;
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("failed to read project directory {}", path.display()))?;
    if !metadata.is_dir() {
        bail!("project path '{}' is not a directory", path.display());
    }
    Ok(path)
}

fn project_config_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".codex").join("config.toml")
}

fn project_catalog_path(project_dir: &Path) -> anyhow::Result<PathBuf> {
    absolutize(&project_dir.join(".codex").join("model-catalog-shim.json"))
}

fn resolve_codex_home(codex_home: Option<&str>) -> anyhow::Result<PathBuf> {
    let path = match codex_home {
        Some(path) => crate::config::expand_tilde(path),
        None => match std::env::var("CODEX_HOME") {
            Ok(path) => crate::config::expand_tilde(&path),
            Err(_) => default_codex_home_dir().ok_or_else(|| {
                anyhow!("could not determine CODEX_HOME; set codex_home or CODEX_HOME")
            })?,
        },
    };
    absolutize(&path)
}

fn default_codex_home_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".codex"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn absolutize(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn parse_toml_document(existing: &str, path: &Path) -> anyhow::Result<DocumentMut> {
    if existing.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        existing
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", path.display()))
    }
}

fn write_toml_document(path: &Path, rendered: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if path.exists() {
        rotate_config_backups(path)?;
    }

    std::fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))
}

fn merge_codex_config_document(
    doc: &mut DocumentMut,
    provider_id: &str,
    model: &str,
    catalog_path: &Path,
    base_url: &str,
    env_key: Option<&str>,
    web_search: Option<&str>,
) -> anyhow::Result<()> {
    if provider_id.trim().is_empty() {
        bail!("provider_id must not be empty");
    }
    if matches!(
        provider_id,
        "openai" | "ollama" | "lmstudio" | "amazon-bedrock"
    ) {
        bail!(
            "provider_id '{}' is reserved by Codex; choose a different custom provider ID",
            provider_id
        );
    }

    let web_search = match web_search {
        Some(mode @ ("disabled" | "cached" | "live")) => Some(mode),
        Some(other) => bail!(
            "invalid web_search mode '{}'; expected one of: disabled, cached, live",
            other
        ),
        None => None,
    };

    doc["model_provider"] = value(provider_id);
    doc["model"] = value(model);
    doc["model_catalog_json"] = value(catalog_path.to_string_lossy().to_string());

    if let Some(mode) = web_search {
        doc["web_search"] = value(mode);
    } else if !doc.as_table().contains_key("web_search") {
        doc["web_search"] = value("disabled");
    }

    let model_providers = ensure_table(doc.as_table_mut(), "model_providers", "model_providers")?;
    let provider_path = format!("model_providers.{provider_id}");
    let provider = ensure_table(model_providers, provider_id, &provider_path)?;

    if env_key.is_some() && provider.contains_key("auth") {
        bail!(
            "{} already contains an auth block. Remove it or use a different provider ID before installing env_key-based shim auth.",
            provider_path
        );
    }

    let _ = provider.remove("model_catalog_json");
    provider["name"] = value("codex-shim");
    provider["base_url"] = value(base_url);
    provider["wire_api"] = value("responses");
    provider["supports_websockets"] = value(false);
    match env_key {
        Some(env_key) => {
            provider["env_key"] = value(env_key);
            provider["env_key_instructions"] =
                value(format!("Set {env_key} before starting Codex."));
        }
        None => {
            let _ = provider.remove("env_key");
            let _ = provider.remove("env_key_instructions");
        }
    }
    Ok(())
}

fn rotate_config_backups(path: &Path) -> anyhow::Result<()> {
    for index in (0..=3).rev() {
        let src = backup_path(path, index);
        if !src.exists() {
            continue;
        }
        if index == 3 {
            std::fs::remove_file(&src)
                .with_context(|| format!("failed to remove old backup {}", src.display()))?;
            continue;
        }
        let dst = backup_path(path, index + 1);
        if dst.exists() {
            std::fs::remove_file(&dst)
                .with_context(|| format!("failed to replace backup {}", dst.display()))?;
        }
        std::fs::rename(&src, &dst).with_context(|| {
            format!(
                "failed to rotate backup {} to {}",
                src.display(),
                dst.display()
            )
        })?;
    }

    let backup0 = backup_path(path, 0);
    std::fs::copy(path, &backup0).with_context(|| {
        format!(
            "failed to create config backup {} from {}",
            backup0.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn backup_path(path: &Path, index: u8) -> PathBuf {
    PathBuf::from(format!("{}.bak.{}", path.display(), index))
}

fn ensure_table<'a>(
    parent: &'a mut Table,
    key: &str,
    full_path: &str,
) -> anyhow::Result<&'a mut Table> {
    if parent.get(key).is_none() {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("{full_path} must be a TOML table"))
}

fn update_project_trust_entry(global_config_path: &Path, project_dir: &Path) -> anyhow::Result<()> {
    let existing = match std::fs::read_to_string(global_config_path) {
        Ok(content) => Some(content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read {}", global_config_path.display()));
        }
    };

    let mut doc = parse_toml_document(existing.as_deref().unwrap_or_default(), global_config_path)?;
    let projects = ensure_table(doc.as_table_mut(), "projects", "projects")?;
    let project_key = project_dir.to_string_lossy();
    let entry = ensure_table(projects, project_key.as_ref(), "projects.<path>")?;
    entry["trust_level"] = value("trusted");

    let mut rendered = doc.to_string();
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    write_toml_document(global_config_path, &rendered)?;
    Ok(())
}

fn build_desktop_doctor_report(
    config: &Config,
    caps: &ProviderCapabilities,
    project_dir: &Path,
    codex_home: Option<&str>,
    provider_id: &str,
) -> anyhow::Result<DesktopDoctorReport> {
    let mut checks = Vec::new();
    let project_config = project_config_path(project_dir);
    let expected_catalog_path = project_catalog_path(project_dir)?;
    let expected_catalog = build_model_catalog(&config.models.catalog, caps);
    let expected_search_support = catalog_supports_web_search(caps, &config.models.catalog);

    if provider_id != default_provider_id() {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "provider_id".into(),
            detail: format!(
                "desktop project support requires provider_id '{}'; got '{}'",
                default_provider_id(),
                provider_id
            ),
        });
    } else {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Supported,
            subject: "provider_id".into(),
            detail: "desktop project installs keep a stable provider identity for shim-managed history/resume".into(),
        });
    }

    if project_config.exists() {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Supported,
            subject: "project_config".into(),
            detail: format!("found project config at {}", project_config.display()),
        });

        let project_toml = std::fs::read_to_string(&project_config)
            .with_context(|| format!("failed to read {}", project_config.display()))?;
        let project_doc = project_toml
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", project_config.display()))?;

        let actual_provider = project_doc["model_provider"].as_str();
        if actual_provider == Some(provider_id) {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "project_model_provider".into(),
                detail: format!("project config uses stable provider '{}'", provider_id),
            });
        } else {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model_provider".into(),
                detail: format!(
                    "project config model_provider is {:?}; expected '{}'",
                    actual_provider, provider_id
                ),
            });
        }

        let active_model = project_doc["model"].as_str();
        match active_model {
            Some(model) if config.models.catalog.iter().any(|spec| spec.slug == model) => {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "project_model".into(),
                    detail: format!(
                        "active project model '{}' is present in shim models.catalog",
                        model
                    ),
                });
            }
            Some(model) => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model".into(),
                detail: format!(
                    "project config model '{}' is not present in shim models.catalog",
                    model
                ),
            }),
            None => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model".into(),
                detail: "project config is missing top-level model".into(),
            }),
        }

        let actual_catalog = project_doc["model_catalog_json"].as_str();
        if actual_catalog == Some(expected_catalog_path.to_string_lossy().as_ref()) {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "project_model_catalog_path".into(),
                detail: format!(
                    "project config points at stable in-project catalog {}",
                    expected_catalog_path.display()
                ),
            });
        } else {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model_catalog_path".into(),
                detail: format!(
                    "project config model_catalog_json is {:?}; expected {}",
                    actual_catalog,
                    expected_catalog_path.display()
                ),
            });
        }

        let provider = &project_doc["model_providers"][provider_id];
        if provider.is_none() {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_provider_block".into(),
                detail: format!(
                    "missing [model_providers.{}] block in project config",
                    provider_id
                ),
            });
        } else {
            let wire_api = provider["wire_api"].as_str();
            if wire_api == Some("responses") {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "wire_api".into(),
                    detail: "project provider uses wire_api = \"responses\"".into(),
                });
            } else {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Unsupported,
                    subject: "wire_api".into(),
                    detail: format!(
                        "project provider wire_api is {:?}; expected \"responses\"",
                        wire_api
                    ),
                });
            }

            let supports_websockets = provider["supports_websockets"].as_bool();
            if supports_websockets == Some(false) {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "supports_websockets".into(),
                    detail: "project provider keeps supports_websockets = false".into(),
                });
            } else {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Unsupported,
                    subject: "supports_websockets".into(),
                    detail: format!(
                        "project provider supports_websockets is {:?}; expected false",
                        supports_websockets
                    ),
                });
            }
        }

        let web_search = project_doc["web_search"].as_str();
        match web_search {
            Some("disabled") => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "web_search".into(),
                detail: "top-level web_search is disabled".into(),
            }),
            Some(mode @ ("cached" | "live")) if expected_search_support => {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "web_search".into(),
                    detail: format!(
                        "top-level web_search = '{}' is consistent with the shim profile and catalog",
                        mode
                    ),
                });
            }
            Some(mode @ ("cached" | "live")) => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "web_search".into(),
                detail: format!(
                    "top-level web_search = '{}' but the active shim profile/catalog does not advertise hosted web search for every model",
                    mode
                ),
            }),
            Some(other) => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "web_search".into(),
                detail: format!(
                    "top-level web_search = '{}' is invalid; expected disabled, cached, or live",
                    other
                ),
            }),
            None => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "web_search".into(),
                detail: "project config is missing top-level web_search".into(),
            }),
        }
    } else {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "project_config".into(),
            detail: format!(
                "missing project config {}; run Codex integration apply for '{}'",
                project_config.display(),
                project_dir.display()
            ),
        });
    }

    if expected_catalog_path.exists() {
        let actual_catalog =
            std::fs::read_to_string(&expected_catalog_path).with_context(|| {
                format!(
                    "failed to read project catalog {}",
                    expected_catalog_path.display()
                )
            })?;
        let actual_catalog: ModelsResponse =
            serde_json::from_str(&actual_catalog).with_context(|| {
                format!(
                    "failed to parse project catalog {}",
                    expected_catalog_path.display()
                )
            })?;
        if actual_catalog == expected_catalog {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "project_catalog".into(),
                detail: "project catalog matches the current shim config and provider profile"
                    .into(),
            });
        } else {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_catalog".into(),
                detail: "project catalog differs from the current shim config or provider profile"
                    .into(),
            });
        }
    } else {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "project_catalog".into(),
            detail: format!(
                "missing project catalog {}",
                expected_catalog_path.display()
            ),
        });
    }

    match resolve_codex_home(codex_home) {
        Ok(codex_home) => {
            let global_config = codex_home.join("config.toml");
            if global_config.exists() {
                let global_toml = std::fs::read_to_string(&global_config)
                    .with_context(|| format!("failed to read {}", global_config.display()))?;
                let global_doc = global_toml
                    .parse::<DocumentMut>()
                    .with_context(|| format!("failed to parse {}", global_config.display()))?;
                let trust_level =
                    global_doc["projects"][project_dir.to_string_lossy().as_ref()]["trust_level"]
                        .as_str();
                if trust_level == Some("trusted") {
                    checks.push(DesktopCheck {
                        status: DesktopCheckStatus::Supported,
                        subject: "project_trust".into(),
                        detail: format!(
                            "global Codex config trusts project '{}'",
                            project_dir.display()
                        ),
                    });
                } else {
                    checks.push(DesktopCheck {
                        status: DesktopCheckStatus::Unsupported,
                        subject: "project_trust".into(),
                        detail: format!(
                            "global Codex config does not mark '{}' as trusted",
                            project_dir.display()
                        ),
                    });
                }
            } else {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Unsupported,
                    subject: "project_trust".into(),
                    detail: format!(
                        "missing global Codex config {}; desktop cannot trust project-scoped config without it",
                        global_config.display()
                    ),
                });
            }
        }
        Err(err) => checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "project_trust".into(),
            detail: format!("could not resolve CODEX_HOME for trust validation: {err}"),
        }),
    }

    checks.push(DesktopCheck {
        status: DesktopCheckStatus::Gated,
        subject: "legacy_non_shim_threads".into(),
        detail: "old non-shim desktop threads may still fail to resume with their original provider context; this depends on Codex desktop thread restoration behavior rather than codex-shim".into(),
    });

    Ok(DesktopDoctorReport { checks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Config {
        parse_config_text(
            r#"
server:
  listen: "127.0.0.1:8787"
  base_path: "/v1"

upstream:
  base_url: "https://api.deepseek.com"
  api_key_env: "DEEPSEEK_API_KEY"

provider:
  kind: deepseek-chat
  profile_config:
    profile: deepseek-chat

models:
  default: "deepseek-v4-pro"
  catalog:
    - slug: "deepseek-v4-pro"
      context_window: 131072

state:
  backend: memory
"#,
        )
        .expect("sample config")
    }

    #[test]
    fn preview_preserves_unmanaged_toml_fields() {
        let config = sample_config();
        let dir = std::env::temp_dir().join(format!("codex-preview-{}", std::process::id()));
        let preview = preview_codex_integration(
            &config,
            &CodexIntegrationOptions {
                project_dir: None,
                codex_home: Some(dir.display().to_string()),
                base_toml_override: Some(
                    r#"
web_search = "live"
custom_flag = true
[model_providers.other]
name = "Other"
"#
                    .into(),
                ),
                ..Default::default()
            },
        )
        .expect("preview");

        assert!(preview.merged_toml.contains("custom_flag = true"));
        assert!(preview.merged_toml.contains("[model_providers.other]"));
        assert!(
            preview
                .merged_toml
                .contains("model_provider = \"codex_shim\"")
        );
        assert!(preview.merged_toml.contains("web_search = \"disabled\""));
    }

    #[test]
    fn preview_uses_project_scoped_paths() {
        let config = sample_config();
        let project_dir =
            std::env::temp_dir().join(format!("codex-project-{}", std::process::id()));
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let preview = preview_codex_integration(
            &config,
            &CodexIntegrationOptions {
                project_dir: Some(project_dir.display().to_string()),
                trust_project: true,
                ..Default::default()
            },
        )
        .expect("preview");

        assert_eq!(preview.mode, "project");
        assert_eq!(
            PathBuf::from(&preview.target_path),
            absolutize(&project_dir.join(".codex").join("config.toml")).expect("target path")
        );
        assert_eq!(
            PathBuf::from(&preview.catalog_path),
            absolutize(&project_dir.join(".codex").join("model-catalog-shim.json"))
                .expect("catalog path")
        );
        assert!(preview.trust_target_path.is_some());
    }
}
