use cronymax::ai::client::ollama_manager::{
    LocalModel, LocalModelDetails, OllamaManager, PullProgress, PullStatus,
};

#[test]
fn test_local_model_deserialize() {
    let json = r#"{
        "name": "llama3:latest",
        "size": 4109842800,
        "modified_at": "2024-01-01T00:00:00Z",
        "digest": "abc123",
        "details": {
            "family": "llama",
            "parameter_size": "8B",
            "quantization_level": "Q4_0"
        }
    }"#;

    let model: LocalModel = serde_json::from_str(json).unwrap();
    assert_eq!(model.name, "llama3:latest");
    assert_eq!(model.size, 4_109_842_800);
    assert_eq!(model.family(), "llama");
    assert_eq!(model.parameter_size(), "8B");
    assert_eq!(model.quantization_level(), "Q4_0");
}

#[test]
fn test_local_model_defaults() {
    // Minimal JSON — all optional fields should default.
    let json = r#"{"name": "codellama"}"#;
    let model: LocalModel = serde_json::from_str(json).unwrap();
    assert_eq!(model.name, "codellama");
    assert_eq!(model.size, 0);
    assert_eq!(model.digest, "");
    assert_eq!(model.family(), "");
    assert_eq!(model.parameter_size(), "");
    assert_eq!(model.quantization_level(), "");
}

#[test]
fn test_local_model_details_default() {
    let details = LocalModelDetails::default();
    assert_eq!(details.family, "");
    assert_eq!(details.parameter_size, "");
    assert_eq!(details.quantization_level, "");
}

#[test]
fn test_pull_status_variants() {
    // Just verify the enum variants exist and can be constructed.
    let _ = PullStatus::PullingManifest;
    let _ = PullStatus::Downloading {
        digest: "sha256:abc".into(),
        total: 1000,
        completed: 500,
    };
    let _ = PullStatus::Verifying;
    let _ = PullStatus::WritingManifest;
    let _ = PullStatus::Success;
    let _ = PullStatus::Failed("network error".into());
}

#[test]
fn test_pull_progress_construction() {
    let progress = PullProgress {
        model_name: "llama3:latest".into(),
        status: PullStatus::Downloading {
            digest: "sha256:abc".into(),
            total: 4_000_000_000,
            completed: 2_000_000_000,
        },
    };
    assert_eq!(progress.model_name, "llama3:latest");
    if let PullStatus::Downloading {
        total, completed, ..
    } = &progress.status
    {
        assert_eq!(*total, 4_000_000_000);
        assert_eq!(*completed, 2_000_000_000);
    } else {
        panic!("Expected Downloading status");
    }
}

#[test]
fn test_ollama_manager_new() {
    // Just verify the manager can be constructed.
    let _manager = OllamaManager::default();
}

#[test]
fn test_tags_response_deserialize() {
    // Test the expected format from GET /api/tags.
    let json = r#"{
        "models": [
            {
                "name": "llama3:latest",
                "size": 4109842800,
                "details": {
                    "family": "llama",
                    "parameter_size": "8B",
                    "quantization_level": "Q4_0"
                }
            },
            {
                "name": "codellama:7b",
                "size": 3800000000,
                "details": {
                    "family": "llama",
                    "parameter_size": "7B",
                    "quantization_level": "Q4_0"
                }
            }
        ]
    }"#;

    // Parse as generic JSON to verify structure matches expectation.
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let models = v["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);

    // Parse individual models.
    let m1: LocalModel = serde_json::from_value(models[0].clone()).unwrap();
    assert_eq!(m1.name, "llama3:latest");
    let m2: LocalModel = serde_json::from_value(models[1].clone()).unwrap();
    assert_eq!(m2.name, "codellama:7b");
}
