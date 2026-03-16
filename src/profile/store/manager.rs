use super::*;

pub struct ProfileManager {
    pub profiles: HashMap<String, Profile>,
    pub active_profile_id: Option<String>,
    pub base_dir: PathBuf,
    /// In-memory store for the active profile.
    pub active_memory: Option<MemoryStore>,
    pub memory_config: MemoryConfig,
}

impl ProfileManager {
    /// Load all profiles from the base directory. Creates "default" if none exist.
    pub fn load(base_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&base_dir)?;

        let mut profiles = HashMap::new();

        // Scan for profile directories.
        if let Ok(entries) = std::fs::read_dir(&base_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let id = entry.file_name().to_string_lossy().to_string();
                    let profile_path = entry.path().join(PROFILE_TOML);
                    if profile_path.exists() {
                        match std::fs::read_to_string(&profile_path) {
                            Ok(contents) => {
                                match toml::from_str::<Profile>(&contents) {
                                    Ok(mut profile) => {
                                        // Back-fill id/name from directory name if empty.
                                        if profile.id.is_empty() {
                                            profile.id = id.clone();
                                        }
                                        if profile.name.is_empty() {
                                            profile.name = id.clone();
                                        }
                                        log::info!("Loaded profile '{}'", id,);
                                        profiles.insert(id, profile);
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "Failed to parse profile {}: {}",
                                            profile_path.display(),
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to read {}: {}", profile_path.display(), e);
                            }
                        }
                    }
                }
            }
        }

        // Create default profile if none exist.
        if profiles.is_empty() {
            let default_profile = Profile {
                id: "default".into(),
                name: "Default".into(),
                sandbox_policy: None,
                sandbox: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                memory: MemoryConfig::default(),
                allowed_skills: default_allowed_skills(),
            };

            let profile_dir = base_dir.join("default");
            std::fs::create_dir_all(profile_dir.join(WEBDATA_DIR))?;
            let profile_file = profile_dir.join(PROFILE_TOML);
            // Only write default config if no file exists — never overwrite user edits.
            if !profile_file.exists() {
                let toml_str = toml::to_string_pretty(&default_profile)?;
                std::fs::write(&profile_file, toml_str)?;
                log::info!("Created default profile at {}", profile_file.display());
            } else {
                log::warn!(
                    "Default profile.toml exists but failed to parse — check {}",
                    profile_file.display()
                );
            }

            profiles.insert("default".into(), default_profile);
        }

        let active_id = profiles.keys().next().cloned();
        let memory_config = active_id
            .as_ref()
            .and_then(|id| profiles.get(id))
            .map(|p| p.memory.clone())
            .unwrap_or_default();

        let mut mgr = Self {
            profiles,
            active_profile_id: active_id.clone(),
            base_dir,
            active_memory: None,
            memory_config,
        };

        // Load memory for active profile.
        if let Some(id) = &active_id {
            mgr.active_memory = Some(mgr.load_memory(id).unwrap_or_else(|_| MemoryStore::new(id)));
        }

        Ok(mgr)
    }

    /// Get the profile directory for a given profile ID.
    pub fn profile_dir(&self, id: &str) -> PathBuf {
        self.base_dir.join(id)
    }

    /// Get the webdata directory for a given profile.
    pub fn webdata_dir(&self, id: &str) -> PathBuf {
        self.profile_dir(id).join(WEBDATA_DIR)
    }

    /// Create a new profile with the given name and LLM config.
    pub fn create_profile(&mut self, name: &str) -> anyhow::Result<&Profile> {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let profile = Profile {
            id: id.clone(),
            name: name.into(),
            sandbox_policy: None,
            sandbox: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            memory: MemoryConfig::default(),
            allowed_skills: default_allowed_skills(),
        };

        // Create disk layout.
        let profile_dir = self.profile_dir(&id);
        std::fs::create_dir_all(profile_dir.join(WEBDATA_DIR))?;

        // Write profile.toml.
        let toml_str = toml::to_string_pretty(&profile)?;
        std::fs::write(profile_dir.join(PROFILE_TOML), toml_str)?;

        self.profiles.insert(id.clone(), profile);
        Ok(self.profiles.get(&id).unwrap())
    }

    /// Delete a profile and all its data.
    /// Returns an error if it's the last remaining profile.
    pub fn delete_profile(&mut self, id: &str) -> anyhow::Result<()> {
        if self.profiles.len() <= 1 {
            anyhow::bail!("Cannot delete the last remaining profile");
        }
        if self.active_profile_id.as_deref() == Some(id) {
            anyhow::bail!("Cannot delete the active profile");
        }
        let dir = self.profile_dir(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        self.profiles.remove(id);
        Ok(())
    }

    /// Edit an existing profile's fields and persist to disk.
    pub fn edit_profile(
        &mut self,
        id: &str,
        name: Option<&str>,
        sandbox: Option<crate::sandbox::policy::SandboxPolicy>,
    ) -> anyhow::Result<()> {
        let profile = self
            .profiles
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("Profile '{}' not found", id))?;
        if let Some(n) = name {
            profile.name = n.to_string();
        }
        if let Some(s) = sandbox {
            profile.sandbox = Some(s);
        }
        let profile_clone = profile.clone();
        self.save_profile(&profile_clone)?;
        Ok(())
    }

    /// Duplicate an existing profile with a new name.
    pub fn duplicate_profile(&mut self, source_id: &str, new_name: &str) -> anyhow::Result<String> {
        let source = self
            .profiles
            .get(source_id)
            .ok_or_else(|| anyhow::anyhow!("Profile '{}' not found", source_id))?
            .clone();
        let new_profile = self.create_profile(new_name)?;
        let new_id = new_profile.id.clone();
        // Copy sandbox policy if present.
        if (source.sandbox.is_some() || source.sandbox_policy.is_some())
            && let Some(p) = self.profiles.get_mut(&new_id)
        {
            p.sandbox = source.sandbox.clone();
            p.sandbox_policy = source.sandbox_policy.clone();
            let p_clone = p.clone();
            self.save_profile(&p_clone)?;
        }
        Ok(new_id)
    }

    /// Set the active profile.
    pub fn set_active(&mut self, id: &str) -> anyhow::Result<()> {
        if !self.profiles.contains_key(id) {
            anyhow::bail!("Profile '{}' not found", id);
        }
        self.active_profile_id = Some(id.to_string());
        self.memory_config = self
            .profiles
            .get(id)
            .map(|p| p.memory.clone())
            .unwrap_or_default();
        self.active_memory = Some(
            self.load_memory(id)
                .unwrap_or_else(|_| MemoryStore::new(id)),
        );
        Ok(())
    }

    /// Get the active profile.
    pub fn active(&self) -> Option<&Profile> {
        self.active_profile_id
            .as_ref()
            .and_then(|id| self.profiles.get(id))
    }

    /// List all profiles.
    pub fn list(&self) -> Vec<&Profile> {
        self.profiles.values().collect()
    }

    /// Save a profile to disk.
    pub fn save_profile(&self, profile: &Profile) -> anyhow::Result<()> {
        let dir = self.profile_dir(&profile.id);
        std::fs::create_dir_all(&dir)?;
        let toml_str = toml::to_string_pretty(profile)?;
        std::fs::write(dir.join(PROFILE_TOML), toml_str)?;
        Ok(())
    }

    /// Load a profile from disk.
    pub fn load_profile(&self, id: &str) -> anyhow::Result<Profile> {
        let path = self.profile_dir(id).join(PROFILE_TOML);
        let contents = std::fs::read_to_string(&path)?;
        let mut profile: Profile = toml::from_str(&contents)?;

        // Migrate legacy category names.
        let mut dirty = false;
        // onboarding → channels
        if let Some(pos) = profile
            .allowed_skills
            .iter()
            .position(|s| s == "onboarding")
        {
            profile.allowed_skills[pos] = "channels".into();
            dirty = true;
        }
        // internal → sandbox (replace "internal" with "sandbox", ensure "chat" present).
        if let Some(pos) = profile.allowed_skills.iter().position(|s| s == "internal") {
            profile.allowed_skills[pos] = "sandbox".into();
            dirty = true;
        }
        // Add "chat" if not present (covers legacy "internal"-only profiles).
        if !profile.allowed_skills.contains(&"chat".to_string()) {
            profile.allowed_skills.push("chat".into());
            dirty = true;
        }
        // Add "sandbox" if not present (covers profiles without either internal or sandbox).
        if !profile.allowed_skills.contains(&"sandbox".to_string()) {
            profile.allowed_skills.push("sandbox".into());
            dirty = true;
        }
        // Add "scheduler" if missing.
        if !profile.allowed_skills.contains(&"scheduler".to_string()) {
            profile.allowed_skills.push("scheduler".into());
            dirty = true;
        }
        if dirty {
            let _ = self.save_profile(&profile);
        }

        Ok(profile)
    }

    /// Save an LLM session to disk (session.json).
    pub fn save_session(&self, session: &LlmSession) -> anyhow::Result<()> {
        let dir = self.profile_dir(&session.profile_id);
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(session)?;
        std::fs::write(dir.join(SESSION_JSON), json)?;
        Ok(())
    }

    /// Load an LLM session from disk.
    pub fn load_session(&self, profile_id: &str) -> anyhow::Result<Option<LlmSession>> {
        let path = self.profile_dir(profile_id).join(SESSION_JSON);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path)?;
        let session: LlmSession = serde_json::from_str(&contents)?;
        Ok(Some(session))
    }

    /// Load the sandbox policy for a profile.
    pub fn sandbox_policy(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<crate::sandbox::policy::SandboxPolicy> {
        let path = self.profile_dir(profile_id).join(POLICY_TOML);
        if path.exists() {
            crate::sandbox::policy::SandboxPolicy::from_toml_file(&path)
        } else {
            Ok(crate::sandbox::policy::SandboxPolicy::from_default())
        }
    }

    /// Get the memory store for the active profile.
    pub fn memory(&self) -> Option<&MemoryStore> {
        self.active_memory.as_ref()
    }

    /// Get mutable memory store for the active profile.
    pub fn memory_mut(&mut self) -> Option<&mut MemoryStore> {
        self.active_memory.as_mut()
    }

    /// Load memory from disk for a profile.
    pub fn load_memory(&self, profile_id: &str) -> anyhow::Result<MemoryStore> {
        let path = self.profile_dir(profile_id).join(MEMORY_JSON);
        if !path.exists() {
            return Ok(MemoryStore::new(profile_id));
        }
        let contents = std::fs::read_to_string(&path)?;
        let store: MemoryStore = serde_json::from_str(&contents)?;
        Ok(store)
    }

    /// Save the active profile's memory to disk.
    pub fn save_memory(&self) -> anyhow::Result<()> {
        if let (Some(id), Some(memory)) = (&self.active_profile_id, &self.active_memory) {
            let dir = self.profile_dir(id);
            std::fs::create_dir_all(&dir)?;
            let json = serde_json::to_string_pretty(memory)?;
            std::fs::write(dir.join(MEMORY_JSON), json)?;
        }
        Ok(())
    }

    /// Reload memory from disk for the active profile.
    pub fn reload_memory(&mut self) -> anyhow::Result<()> {
        if let Some(id) = self.active_profile_id.clone() {
            self.active_memory = Some(self.load_memory(&id)?);
        }
        Ok(())
    }
}
