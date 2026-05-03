#pragma once

#include <string_view>

namespace cronymax {

// Returns the raw SVG content for the given Codicon filename (e.g. "add.svg").
// Data is embedded at compile time by cmake/GenerateIconData.cmake so there is
// no runtime file I/O or bundle resource dependency.
// Returns an empty string_view if the filename is not found.
std::string_view GetIconSvgData(std::string_view filename);

}  // namespace cronymax
