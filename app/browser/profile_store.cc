#include "browser/profile_store.h"

#include <algorithm>
#include <cctype>
#include <fstream>

#include <yaml-cpp/yaml.h>

namespace cronymax {

namespace {

const char* kDefaultId = "default";

const char* kDefaultYaml =
    "# Built-in default profile. Cannot be deleted.\n"
    "id: default\n"
    "name: Default\n"
    "allow_network: true\n"
    "extra_read_paths: []\n"
    "extra_write_paths: []\n"
    "extra_deny_paths: []\n";

}  // namespace

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

ProfileStore::ProfileStore(std::filesystem::path home_dir)
    : profiles_dir_(std::move(home_dir) / ".cronymax" / "profiles") {}

// ---------------------------------------------------------------------------
// EnsureDefaultProfile
// ---------------------------------------------------------------------------

void ProfileStore::EnsureDefaultProfile() {
  if (profiles_dir_.empty()) return;
  std::error_code ec;
  std::filesystem::create_directories(profiles_dir_, ec);
  const auto path = ProfilePath(kDefaultId);
  if (!std::filesystem::exists(path, ec)) {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (out) {
      out << kDefaultYaml;
    }
  }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

std::vector<ProfileRecord> ProfileStore::List() const {
  std::vector<ProfileRecord> result;
  std::error_code ec;
  if (!std::filesystem::is_directory(profiles_dir_, ec)) {
    return result;
  }
  for (const auto& entry :
       std::filesystem::directory_iterator(profiles_dir_, ec)) {
    if (ec) break;
    const auto& p = entry.path();
    if (p.extension() != ".yaml") continue;
    const std::string id = p.stem().string();
    try {
      result.push_back(ParseYaml(p, id));
    } catch (...) {
      // Skip unparseable profiles.
    }
  }
  // Stable order: default first, then alphabetical.
  std::sort(result.begin(), result.end(), [](const ProfileRecord& a, const ProfileRecord& b) {
    if (a.id == kDefaultId) return true;
    if (b.id == kDefaultId) return false;
    return a.id < b.id;
  });
  return result;
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

std::optional<ProfileRecord> ProfileStore::Get(const std::string& id) const {
  const auto path = ProfilePath(id);
  std::error_code ec;
  if (!std::filesystem::exists(path, ec)) {
    return std::nullopt;
  }
  try {
    return ParseYaml(path, id);
  } catch (...) {
    return std::nullopt;
  }
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

ProfileStoreError ProfileStore::Create(const ProfileRules& rules,
                                       std::string* out_id) {
  if (profiles_dir_.empty()) return ProfileStoreError::kIoError;
  const std::string id = Slugify(rules.name);
  if (id.empty()) return ProfileStoreError::kIoError;

  const auto path = ProfilePath(id);
  std::error_code ec;
  std::filesystem::create_directories(profiles_dir_, ec);
  if (std::filesystem::exists(path, ec)) {
    return ProfileStoreError::kAlreadyExists;
  }

  ProfileRecord rec;
  rec.id                = id;
  rec.name              = rules.name;
  rec.allow_network     = rules.allow_network;
  rec.extra_read_paths  = rules.extra_read_paths;
  rec.extra_write_paths = rules.extra_write_paths;
  rec.extra_deny_paths  = rules.extra_deny_paths;

  try {
    WriteYaml(path, rec);
  } catch (...) {
    return ProfileStoreError::kIoError;
  }

  if (out_id) *out_id = id;
  return ProfileStoreError::kOk;
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

ProfileStoreError ProfileStore::Update(const std::string& id,
                                       const ProfileRules& rules) {
  if (profiles_dir_.empty()) return ProfileStoreError::kIoError;
  const auto path = ProfilePath(id);
  std::error_code ec;
  if (!std::filesystem::exists(path, ec)) {
    return ProfileStoreError::kNotFound;
  }

  ProfileRecord rec;
  rec.id                = id;
  rec.name              = rules.name;
  rec.allow_network     = rules.allow_network;
  rec.extra_read_paths  = rules.extra_read_paths;
  rec.extra_write_paths = rules.extra_write_paths;
  rec.extra_deny_paths  = rules.extra_deny_paths;

  try {
    WriteYaml(path, rec);
  } catch (...) {
    return ProfileStoreError::kIoError;
  }
  return ProfileStoreError::kOk;
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

ProfileStoreError ProfileStore::Delete(
    const std::string& id,
    const std::vector<std::string>& space_profile_ids) {
  if (profiles_dir_.empty()) return ProfileStoreError::kIoError;
  if (id == kDefaultId) return ProfileStoreError::kCannotDeleteDefault;

  for (const auto& pid : space_profile_ids) {
    if (pid == id) return ProfileStoreError::kInUse;
  }

  const auto path = ProfilePath(id);
  std::error_code ec;
  if (!std::filesystem::exists(path, ec)) {
    return ProfileStoreError::kNotFound;
  }

  std::filesystem::remove(path, ec);
  return ec ? ProfileStoreError::kIoError : ProfileStoreError::kOk;
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

std::filesystem::path ProfileStore::ProfilePath(const std::string& id) const {
  return profiles_dir_ / (id + ".yaml");
}

ProfileRecord ProfileStore::ParseYaml(const std::filesystem::path& path,
                                       const std::string& id) const {
  YAML::Node doc = YAML::LoadFile(path.string());
  ProfileRecord rec;
  rec.id = id;
  rec.name = doc["name"] ? doc["name"].as<std::string>() : id;
  rec.allow_network = doc["allow_network"] ? doc["allow_network"].as<bool>() : true;

  auto load_list = [&](const char* key, std::vector<std::string>& out) {
    if (doc[key] && doc[key].IsSequence()) {
      for (const auto& item : doc[key]) {
        const std::string v = item.as<std::string>();
        if (!v.empty()) out.push_back(v);
      }
    }
  };
  load_list("extra_read_paths",  rec.extra_read_paths);
  load_list("extra_write_paths", rec.extra_write_paths);
  load_list("extra_deny_paths",  rec.extra_deny_paths);
  return rec;
}

void ProfileStore::WriteYaml(const std::filesystem::path& path,
                              const ProfileRecord& rec) const {
  YAML::Emitter out;
  out << YAML::BeginMap;
  out << YAML::Key << "id"            << YAML::Value << rec.id;
  out << YAML::Key << "name"          << YAML::Value << rec.name;
  out << YAML::Key << "allow_network" << YAML::Value << rec.allow_network;

  auto emit_list = [&](const char* key, const std::vector<std::string>& v) {
    out << YAML::Key << key << YAML::Value << YAML::BeginSeq;
    for (const auto& s : v) out << s;
    out << YAML::EndSeq;
  };
  emit_list("extra_read_paths",  rec.extra_read_paths);
  emit_list("extra_write_paths", rec.extra_write_paths);
  emit_list("extra_deny_paths",  rec.extra_deny_paths);
  out << YAML::EndMap;

  std::ofstream f(path, std::ios::binary | std::ios::trunc);
  if (!f) throw std::runtime_error("cannot open for write: " + path.string());
  f << out.c_str();
  f.close();
  if (!f) throw std::runtime_error("write failed: " + path.string());
}

// static
std::string ProfileStore::Slugify(const std::string& name) {
  std::string slug;
  slug.reserve(name.size());
  bool prev_dash = false;
  for (char c : name) {
    if (std::isalnum(static_cast<unsigned char>(c))) {
      slug += static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
      prev_dash = false;
    } else if (!slug.empty() && !prev_dash) {
      slug += '-';
      prev_dash = true;
    }
  }
  // Trim trailing dash.
  while (!slug.empty() && slug.back() == '-') slug.pop_back();
  return slug;
}

}  // namespace cronymax
