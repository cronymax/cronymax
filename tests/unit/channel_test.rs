//! Unit tests for the channel subsystem — config parsing, allowlist, UI state,
//! memory store, agent loop pipeline, and skill filtering.

use cronymax::channel::ConnectionState;
use cronymax::channel::config::{ChannelConfig, ClawConfig, LarkChannelConfig};

// ─── ClawConfig defaults ─────────────────────────────────────────────────────

#[test]
fn claw_config_default_disabled() {
    let cfg = ClawConfig::default();
    assert!(!cfg.enabled);
    assert!(cfg.lark.is_none());
}

// ─── LarkChannelConfig validation ────────────────────────────────────────────

#[test]
fn lark_config_valid() {
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_test123".to_string(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec!["ou_abc".to_string()],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn lark_config_empty_app_id() {
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: String::new(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("app_id must not be empty"));
}

#[test]
fn lark_config_bad_app_id_prefix() {
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "app_test123".to_string(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("must start with 'cli_'"));
}

#[test]
fn lark_config_empty_secret_env() {
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: String::new(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("app_secret_env must not be empty"));
}

#[test]
fn lark_config_http_base_rejected() {
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec![],
        api_base: "http://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("HTTPS"));
}

// ─── LarkChannelConfig.resolve_app_secret ────────────────────────────────────

#[test]
fn resolve_secret_from_env() {
    let key = "CRONYMAX_TEST_LARK_SECRET_987";
    unsafe {
        std::env::set_var(key, "s3cret");
    }
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: key.to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let store = cronymax::secret::SecretStore::default();
    assert_eq!(cfg.resolve_app_secret(&store).unwrap(), "s3cret");
    unsafe {
        std::env::remove_var(key);
    }
}

#[test]
fn resolve_secret_missing_env() {
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: "CRONYMAX_NONEXISTENT_VAR_42".to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let store = cronymax::secret::SecretStore::default();
    assert!(cfg.resolve_app_secret(&store).is_err());
}

// ─── Allowlist logic (via LarkChannel::check_authorized) ────────────────────

#[test]
fn lark_channel_allowlist_deny_empty() {
    use cronymax::channel::Channel;
    use cronymax::channel::lark::LarkChannel;

    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec![], // deny all
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let channel = LarkChannel::new(
        cfg,
        std::sync::Arc::new(cronymax::secret::SecretStore::default()),
    );
    assert!(!channel.is_sender_authorized("ou_anyone"));
}

#[test]
fn lark_channel_allowlist_wildcard() {
    use cronymax::channel::Channel;
    use cronymax::channel::lark::LarkChannel;

    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec!["*".to_string()],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let channel = LarkChannel::new(
        cfg,
        std::sync::Arc::new(cronymax::secret::SecretStore::default()),
    );
    assert!(channel.is_sender_authorized("ou_anyone"));
}

#[test]
fn lark_channel_allowlist_specific() {
    use cronymax::channel::Channel;
    use cronymax::channel::lark::LarkChannel;

    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_abc".to_string(),
        app_secret_env: "LARK_SECRET".to_string(),
        allowed_users: vec!["ou_alice".to_string(), "ou_bob".to_string()],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let channel = LarkChannel::new(
        cfg,
        std::sync::Arc::new(cronymax::secret::SecretStore::default()),
    );
    assert!(channel.is_sender_authorized("ou_alice"));
    assert!(channel.is_sender_authorized("ou_bob"));
    assert!(!channel.is_sender_authorized("ou_eve"));
}

// ─── ConnectionState display ─────────────────────────────────────────────────

#[test]
fn connection_state_display() {
    assert_eq!(ConnectionState::Connected.to_string(), "Connected");
    assert_eq!(ConnectionState::Disconnected.to_string(), "Disconnected");
    assert_eq!(ConnectionState::Reconnecting.to_string(), "Reconnecting");
    assert_eq!(ConnectionState::Connecting.to_string(), "Connecting");
    assert_eq!(ConnectionState::Error.to_string(), "Error");
}

// ─── ChannelsSettingsState roundtrip ─────────────────────────────────────────

#[test]
fn channels_ui_state_from_config_none() {
    use cronymax::ui::settings::channels::ChannelsSettingsState;
    let state = ChannelsSettingsState::from_config(None, false);
    assert!(state.app_id.is_empty());
    assert_eq!(state.app_secret_env, "LARK_APP_SECRET");
    assert_eq!(state.api_base, "https://open.feishu.cn");
    assert_eq!(state.connection_state, ConnectionState::Disconnected);
    assert!(!state.lark_enabled);
}

#[test]
fn channels_ui_state_roundtrip() {
    use cronymax::ui::settings::channels::ChannelsSettingsState;
    let cfg = LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_roundtrip".to_string(),
        app_secret_env: "MY_SECRET".to_string(),
        allowed_users: vec!["ou_a".to_string(), "ou_b".to_string()],
        api_base: "https://open.larksuite.com".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    };
    let state = ChannelsSettingsState::from_config(Some(&cfg), true);
    assert_eq!(state.app_id, "cli_roundtrip");
    assert_eq!(state.app_secret_env, "MY_SECRET");
    assert_eq!(state.allowed_users_text, "ou_a, ou_b");
    assert_eq!(state.api_base, "https://open.larksuite.com");
    assert!(state.lark_enabled);

    // Convert back.
    let cfg2 = state.to_lark_config();
    assert_eq!(cfg2.app_id, "cli_roundtrip");
    assert_eq!(cfg2.allowed_users, vec!["ou_a", "ou_b"]);
}

// ─── TOML deserialization ────────────────────────────────────────────────────

#[test]
fn claw_config_toml_deserialize() {
    let toml_str = r#"
enabled = true

[lark]
app_id = "cli_toml_test"
app_secret_env = "LARK_SECRET"
allowed_users = ["ou_x", "ou_y"]
api_base = "https://open.feishu.cn"
"#;
    let cfg: ClawConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.enabled);
    let lark = cfg.lark.unwrap();
    assert_eq!(lark.app_id, "cli_toml_test");
    assert_eq!(lark.allowed_users.len(), 2);
}

#[test]
fn claw_config_toml_defaults() {
    let toml_str = r#"
enabled = false
"#;
    let cfg: ClawConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.enabled);
    assert!(cfg.lark.is_none());
}

// ─── T031: ChannelConfig typed enum deserialization ──────────────────────────

#[test]
fn channel_config_typed_enum_lark() {
    let toml_str = r#"
enabled = true

[[channels]]
type = "lark"
app_id = "cli_enum_test"
app_secret_env = "LARK_SECRET"
allowed_users = ["ou_a"]
api_base = "https://open.feishu.cn"
"#;
    let cfg: ClawConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.enabled);
    assert_eq!(cfg.channels.len(), 1);
    match &cfg.channels[0] {
        ChannelConfig::Lark(lark) => {
            assert_eq!(lark.app_id, "cli_enum_test");
            assert_eq!(lark.allowed_users, vec!["ou_a"]);
        }
    }
}

#[test]
fn channel_config_multiple_channels() {
    let toml_str = r#"
enabled = true

[[channels]]
type = "lark"
app_id = "cli_first"
app_secret_env = "SECRET_1"

[[channels]]
type = "lark"
app_id = "cli_second"
app_secret_env = "SECRET_2"
allowed_users = ["ou_x", "ou_y"]
"#;
    let cfg: ClawConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.channels.len(), 2);
    match &cfg.channels[1] {
        ChannelConfig::Lark(lark) => {
            assert_eq!(lark.app_id, "cli_second");
            assert_eq!(lark.allowed_users.len(), 2);
        }
    }
}

#[test]
fn channel_config_validate_dispatch() {
    let valid = ChannelConfig::Lark(LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_ok".to_string(),
        app_secret_env: "SEC".to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    });
    assert!(valid.validate().is_ok());

    let invalid = ChannelConfig::Lark(LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "".to_string(),
        app_secret_env: "SEC".to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    });
    assert!(invalid.validate().is_err());
}

#[test]
fn channel_config_display_name() {
    let cfg = ChannelConfig::Lark(LarkChannelConfig {
        instance_id: "lark".into(),
        app_id: "cli_x".to_string(),
        app_secret_env: "SEC".to_string(),
        allowed_users: vec![],
        api_base: "https://open.feishu.cn".to_string(),
        profile_id: "default".to_string(),
        secret_storage: Default::default(),
    });
    assert_eq!(cfg.display_name(), "Feishu/Lark");
}

#[test]
fn claw_config_migrate_legacy() {
    let toml_str = r#"
enabled = true

[lark]
app_id = "cli_legacy"
app_secret_env = "LARK_SECRET"
allowed_users = ["ou_legacy"]
api_base = "https://open.feishu.cn"
"#;
    let mut cfg: ClawConfig = toml::from_str(toml_str).unwrap();
    cfg.migrate_legacy();
    // Legacy lark should be consumed.
    assert!(cfg.lark.is_none());
    // Should appear in channels Vec.
    assert_eq!(cfg.channels.len(), 1);
    match &cfg.channels[0] {
        ChannelConfig::Lark(lark) => {
            assert_eq!(lark.app_id, "cli_legacy");
        }
    }
}

#[test]
fn claw_config_migrate_legacy_no_duplicate() {
    let toml_str = r#"
enabled = true

[lark]
app_id = "cli_legacy"
app_secret_env = "SEC"

[[channels]]
type = "lark"
app_id = "cli_existing"
app_secret_env = "SEC"
"#;
    let mut cfg: ClawConfig = toml::from_str(toml_str).unwrap();
    cfg.migrate_legacy();
    // Should NOT add a second lark entry.
    assert_eq!(cfg.channels.len(), 1);
    match &cfg.channels[0] {
        ChannelConfig::Lark(lark) => {
            assert_eq!(lark.app_id, "cli_existing");
        }
    }
}

#[test]
fn lark_config_profile_id_default() {
    let toml_str = r#"
type = "lark"
app_id = "cli_prof"
app_secret_env = "SEC"
"#;
    let cfg: ChannelConfig = toml::from_str(toml_str).unwrap();
    match cfg {
        ChannelConfig::Lark(lark) => {
            assert_eq!(lark.profile_id, "default");
        }
    }
}

#[test]
fn lark_config_profile_id_custom() {
    let toml_str = r#"
type = "lark"
app_id = "cli_prof"
app_secret_env = "SEC"
profile_id = "team-a"
"#;
    let cfg: ChannelConfig = toml::from_str(toml_str).unwrap();
    match cfg {
        ChannelConfig::Lark(lark) => {
            assert_eq!(lark.profile_id, "team-a");
        }
    }
}

// ─── T032: MemoryStore helper functions ──────────────────────────────────────

#[test]
fn cosine_similarity_identical() {
    use cronymax::channel::memory::{bytes_to_embedding, cosine_similarity, embedding_to_bytes};
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    let sim = cosine_similarity(&a, &b);
    assert!(
        (sim - 1.0).abs() < 0.001,
        "identical vectors should have similarity ~1.0, got {sim}"
    );
}

#[test]
fn cosine_similarity_orthogonal() {
    use cronymax::channel::memory::cosine_similarity;
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    let sim = cosine_similarity(&a, &b);
    assert!(
        sim.abs() < 0.001,
        "orthogonal vectors should have similarity ~0.0, got {sim}"
    );
}

#[test]
fn cosine_similarity_opposite() {
    use cronymax::channel::memory::cosine_similarity;
    let a = vec![1.0, 0.0];
    let b = vec![-1.0, 0.0];
    let sim = cosine_similarity(&a, &b);
    assert!(
        (sim + 1.0).abs() < 0.001,
        "opposite vectors should have similarity ~-1.0, got {sim}"
    );
}

#[test]
fn top_k_similar_basic() {
    use cronymax::channel::memory::top_k_similar;
    let query = vec![1.0, 0.0, 0.0];
    let candidates: Vec<(usize, Vec<f32>)> = vec![
        (1, vec![1.0, 0.0, 0.0]), // perfect match
        (2, vec![0.0, 1.0, 0.0]), // orthogonal
        (3, vec![0.7, 0.7, 0.0]), // partial match
    ];
    let result = top_k_similar(&query, &candidates, 2);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, 1); // best match first
    assert_eq!(result[1].0, 3); // second best
}

#[test]
fn top_k_similar_truncates() {
    use cronymax::channel::memory::top_k_similar;
    let query = vec![1.0, 0.0];
    let candidates: Vec<(usize, Vec<f32>)> = vec![
        (1, vec![0.9, 0.1]),
        (2, vec![0.8, 0.2]),
        (3, vec![0.7, 0.3]),
    ];
    let result = top_k_similar(&query, &candidates, 1);
    assert_eq!(result.len(), 1, "should return at most k results");
}

#[test]
fn embedding_roundtrip() {
    use cronymax::channel::memory::{bytes_to_embedding, embedding_to_bytes};
    let original = vec![1.0_f32, -0.5, std::f32::consts::PI, 0.0];
    let bytes = embedding_to_bytes(&original);
    let recovered = bytes_to_embedding(&bytes);
    assert_eq!(original.len(), recovered.len());
    for (a, b) in original.iter().zip(recovered.iter()) {
        assert!((a - b).abs() < 1e-6, "mismatch: {a} vs {b}");
    }
}

// ─── T033: Agent loop pipeline helpers ───────────────────────────────────────

#[test]
fn normalize_message_format() {
    use cronymax::channel::ChannelMessage;
    use cronymax::channel::ReplyTarget;
    use cronymax::channel::agent_loop::normalize_message;

    let msg = ChannelMessage {
        id: "msg_1".into(),
        channel_id: "lark".into(),
        sender_id: "ou_alice".into(),
        sender_name: Some("Alice".into()),
        chat_id: "chat_001".into(),
        content: "Hello world".into(),
        timestamp: 1700000000000,
        reply_target: ReplyTarget {
            channel_id: "lark".into(),
            chat_id: "chat_001".into(),
            message_id: None,
        },
    };
    let normalized = normalize_message(&msg);
    assert!(
        normalized.content.contains("Hello world"),
        "should contain original content"
    );
    assert!(
        normalized.content.contains("lark"),
        "should mention channel"
    );
    assert!(
        normalized.content.contains("Alice"),
        "should mention sender name"
    );
}

#[test]
fn normalize_message_no_sender_name() {
    use cronymax::channel::ChannelMessage;
    use cronymax::channel::ReplyTarget;
    use cronymax::channel::agent_loop::normalize_message;

    let msg = ChannelMessage {
        id: "msg_2".into(),
        channel_id: "slack".into(),
        sender_id: "U12345".into(),
        sender_name: None,
        chat_id: "C_general".into(),
        content: "Test message".into(),
        timestamp: 1700000000000,
        reply_target: ReplyTarget {
            channel_id: "slack".into(),
            chat_id: "C_general".into(),
            message_id: None,
        },
    };
    let normalized = normalize_message(&msg);
    assert!(normalized.content.contains("Test message"));
    assert!(
        normalized.content.contains("U12345"),
        "should use sender_id as fallback"
    );
}

#[test]
fn session_id_stable() {
    use cronymax::channel::agent_loop::session_id_for_channel;
    let id1 = session_id_for_channel("lark", "chat_001");
    let id2 = session_id_for_channel("lark", "chat_001");
    assert_eq!(
        id1, id2,
        "same inputs should always produce same session ID"
    );
    assert!(
        id1 >= 900_000,
        "session IDs should be >= 900_000, got {id1}"
    );
}

#[test]
fn session_id_different_channels() {
    use cronymax::channel::agent_loop::session_id_for_channel;
    let id1 = session_id_for_channel("lark", "chat_001");
    let id2 = session_id_for_channel("slack", "chat_001");
    assert_ne!(
        id1, id2,
        "different channels should produce different session IDs"
    );
}

// ─── T034: SkillRegistry tools_filtered ──────────────────────────────────────

#[test]
fn tools_filtered_empty_allowed() {
    use cronymax::ai::skills::{Skill, SkillRegistry};
    use serde_json::json;
    use std::sync::Arc;

    let mut registry = SkillRegistry::new();
    registry.register(
        Skill {
            name: "test_skill".into(),
            description: "A test".into(),
            parameters_schema: json!({}),
            category: "terminal".into(),
        },
        Arc::new(|_| Box::pin(async { Ok(json!({})) })),
    );
    let tools = registry.to_openai_tools_filtered(&[]);
    assert!(
        tools.is_empty(),
        "empty allowed list should return no tools"
    );
}

#[test]
fn tools_filtered_matching_category() {
    use cronymax::ai::skills::{Skill, SkillRegistry};
    use serde_json::json;
    use std::sync::Arc;

    let mut registry = SkillRegistry::new();
    registry.register(
        Skill {
            name: "run_cmd".into(),
            description: "Run command".into(),
            parameters_schema: json!({"type":"object"}),
            category: "terminal".into(),
        },
        Arc::new(|_| Box::pin(async { Ok(json!({})) })),
    );
    registry.register(
        Skill {
            name: "browse".into(),
            description: "Browse web".into(),
            parameters_schema: json!({"type":"object"}),
            category: "browser".into(),
        },
        Arc::new(|_| Box::pin(async { Ok(json!({})) })),
    );

    let allowed = vec!["terminal".to_string()];
    let tools = registry.to_openai_tools_filtered(&allowed);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["function"]["name"], "run_cmd");
}

#[test]
fn handlers_filtered_matching_category() {
    use cronymax::ai::skills::{Skill, SkillRegistry};
    use serde_json::json;
    use std::sync::Arc;

    let mut registry = SkillRegistry::new();
    registry.register(
        Skill {
            name: "run_cmd".into(),
            description: "Run command".into(),
            parameters_schema: json!({"type":"object"}),
            category: "terminal".into(),
        },
        Arc::new(|_| Box::pin(async { Ok(json!({})) })),
    );
    registry.register(
        Skill {
            name: "browse".into(),
            description: "Browse web".into(),
            parameters_schema: json!({"type":"object"}),
            category: "browser".into(),
        },
        Arc::new(|_| Box::pin(async { Ok(json!({})) })),
    );

    let allowed = vec!["terminal".to_string(), "browser".to_string()];
    let handlers = registry.handlers_filtered(&allowed);
    assert_eq!(handlers.len(), 2);
    assert!(handlers.contains_key("run_cmd"));
    assert!(handlers.contains_key("browse"));
}

#[test]
fn handlers_filtered_no_match() {
    use cronymax::ai::skills::{Skill, SkillRegistry};
    use serde_json::json;
    use std::sync::Arc;

    let mut registry = SkillRegistry::new();
    registry.register(
        Skill {
            name: "run_cmd".into(),
            description: "Run command".into(),
            parameters_schema: json!({"type":"object"}),
            category: "terminal".into(),
        },
        Arc::new(|_| Box::pin(async { Ok(json!({})) })),
    );

    let allowed = vec!["webview".to_string()];
    let handlers = registry.handlers_filtered(&allowed);
    assert!(handlers.is_empty());
}
