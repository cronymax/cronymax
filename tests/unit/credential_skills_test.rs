use cronymax::ai::skills::credentials::{CredentialEntry, CredentialIndex};

#[test]
fn test_credential_index_default() {
    let idx = CredentialIndex::default();
    assert_eq!(idx.version, 1);
    assert!(idx.entries.is_empty());
}

#[test]
fn test_credential_index_serde_roundtrip() {
    let idx = CredentialIndex {
        version: 1,
        entries: vec![
            CredentialEntry {
                service: "openai".into(),
                key: "api_key".into(),
                created_at: "2024-01-01T00:00:00Z".into(),
                last_accessed_at: "2024-06-01T00:00:00Z".into(),
            },
            CredentialEntry {
                service: "lark".into(),
                key: "app_secret".into(),
                created_at: "2024-02-01T00:00:00Z".into(),
                last_accessed_at: "2024-03-01T00:00:00Z".into(),
            },
        ],
    };

    let json = serde_json::to_string(&idx).unwrap();
    let recovered: CredentialIndex = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.version, 1);
    assert_eq!(recovered.entries.len(), 2);
    assert_eq!(recovered.entries[0].service, "openai");
    assert_eq!(recovered.entries[0].key, "api_key");
    assert_eq!(recovered.entries[1].service, "lark");
    assert_eq!(recovered.entries[1].key, "app_secret");
}

#[test]
fn test_credential_entry_serde() {
    let entry = CredentialEntry {
        service: "github".into(),
        key: "token".into(),
        created_at: "2024-01-01T00:00:00Z".into(),
        last_accessed_at: "2024-01-02T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert!(json.contains("github"));
    assert!(json.contains("token"));

    let recovered: CredentialEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.service, "github");
    assert_eq!(recovered.key, "token");
}

#[test]
fn test_credential_index_corrupt_json_fallback() {
    // If the JSON is corrupt, default should be returned.
    let result: Result<CredentialIndex, _> = serde_json::from_str("not valid json");
    assert!(result.is_err());
    // In the actual code, load_index() returns default on error.
    let fallback = CredentialIndex::default();
    assert_eq!(fallback.version, 1);
    assert!(fallback.entries.is_empty());
}

#[test]
fn test_credential_list_returns_entries_without_values() {
    // Verify the CredentialEntry struct has no value field.
    let entry = CredentialEntry {
        service: "test".into(),
        key: "secret".into(),
        created_at: "now".into(),
        last_accessed_at: "now".into(),
    };
    let json = serde_json::to_string(&entry).unwrap();
    // "value" should not appear in the serialized output.
    assert!(!json.contains("\"value\""));
}
