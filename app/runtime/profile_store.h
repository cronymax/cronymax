#pragma once

#include <filesystem>
#include <optional>
#include <string>
#include <vector>

namespace cronymax {

/// A sandbox policy profile stored as ~/.cronymax/profiles/<id>.yaml.
/// `id` is the kebab-case filename stem; immutable after creation.
/// `name` is the mutable display name.
struct ProfileRecord {
  std::string id;    // e.g. "locked-work"
  std::string name;  // e.g. "Locked Work"
  // Logical memory namespace id for runtime memory storage. This can differ
  // from profile id to support hot-swapped memory backends per profile.
  std::string memory_id;
  bool allow_network = true;
  std::vector<std::string> extra_read_paths;
  std::vector<std::string> extra_write_paths;
  std::vector<std::string> extra_deny_paths;
};

/// Rules used to mutate a profile (passed to Create / Update).
struct ProfileRules {
  std::string name;
  // Optional override for ProfileRecord.memory_id. Empty means default to
  // profile id on create, or keep existing value on update.
  std::string memory_id;
  bool allow_network = true;
  std::vector<std::string> extra_read_paths;
  std::vector<std::string> extra_write_paths;
  std::vector<std::string> extra_deny_paths;
};

/// Error codes returned by mutable ProfileStore operations.
enum class ProfileStoreError {
  kOk,
  kAlreadyExists,        // Create: slug collision
  kNotFound,             // Update / Delete / Get: no such profile
  kCannotDeleteDefault,  // Delete: id == "default"
  kInUse,                // Delete: spaces reference this profile
  kIoError,              // file read/write failure
};

/// Manages named sandbox profiles stored as YAML files in
/// ~/.cronymax/profiles/.  All methods are synchronous and suitable for
/// calling from the UI thread (profiles are small; I/O is negligible).
class ProfileStore {
 public:
  /// Default constructor — creates an empty (no-op) store. All methods
  /// return empty / error results when `profiles_dir_` is unset.
  /// Used by SpaceManager before Init() is called.
  ProfileStore() = default;

  /// Initialise with the user's home directory. Profiles are read from /
  /// written to `home_dir/.cronymax/profiles/`.
  explicit ProfileStore(std::filesystem::path home_dir);

  /// Ensure ~/.cronymax/profiles/default.yaml exists; write it if absent.
  /// Should be called once at app startup.
  void EnsureDefaultProfile();

  /// List all profiles found in the profiles directory.
  std::vector<ProfileRecord> List() const;

  /// Load a single profile by ID. Returns nullopt if not found.
  std::optional<ProfileRecord> Get(const std::string& id) const;

  /// Create a new profile. Slugifies `rules.name` to derive the profile ID.
  /// Returns kAlreadyExists if a file with the same slug already exists.
  ProfileStoreError Create(const ProfileRules& rules,
                           std::string* out_id = nullptr);

  /// Update the rules of an existing profile (ID unchanged).
  ProfileStoreError Update(const std::string& id, const ProfileRules& rules);

  /// Delete a profile file. Blocked on `default` and if any space references
  /// it. `space_profile_ids` should be the current list of profile_id values
  /// from the spaces table; the caller is responsible for querying them.
  ProfileStoreError Delete(const std::string& id,
                           const std::vector<std::string>& space_profile_ids);

  const std::filesystem::path& profiles_dir() const { return profiles_dir_; }

 private:
  std::filesystem::path ProfilePath(const std::string& id) const;
  ProfileRecord ParseYaml(const std::filesystem::path& path,
                          const std::string& id) const;
  void WriteYaml(const std::filesystem::path& path,
                 const ProfileRecord& rec) const;
  static std::string Slugify(const std::string& name);

  std::filesystem::path profiles_dir_;
};

}  // namespace cronymax
