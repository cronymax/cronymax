// SPDX-License-Identifier: Apache-2.0
//
// Extension webview custom scheme: `cronymax-webview://<ext-id>/<path>`.
//
// Each extension that creates a webview panel via `cronymax.window.
// createWebviewPanel(...)` gets its iframe loaded from a URL of the
// form `cronymax-webview://<ext-id>/<path>`. The scheme handler below
// resolves the URL to a file inside `~/.cronymax/extensions/<ext-id>/`
// and serves the bytes with a strict CSP header.
//
// Origin model:
//   * The scheme is registered as **standard** + **secure** +
//     **CSP-bypassing** so the iframe gets its own origin scoped to
//     `<ext-id>` (cross-extension `postMessage` is blocked by browser
//     origin rules without any extra work on our side).
//   * It is intentionally NOT marked `local` — that flag would also
//     gate the scheme behind file:// permissions and break the iframe
//     from a built-in (file://) chrome.
//
// Path safety:
//   * The relative path is canonicalised and rejected if it escapes the
//     extension's root directory (`../` traversal).
//   * Symlinks are followed during canonicalisation; if the final
//     resolved path is outside the extension root the request returns
//     403 Forbidden.
//
// CSP:
//   * `default-src 'none'; script-src 'self'; style-src 'self'
//     'unsafe-inline'; img-src cronymax-webview: data:;
//     connect-src 'self'; font-src 'self' data:`
//     — the iframe can only run scripts / load styles from its own
//     extension's directory; remote network calls (`fetch`, ws) are
//     blocked. Extensions that need network reach do so via their
//     Node-side `main.js` and `postMessage` the data into the iframe.

#pragma once

#include <string>

#include "include/cef_scheme.h"

namespace cronymax {

/// Custom scheme name. Used at registration (`OnRegisterCustomSchemes`)
/// and at factory bind (`CefRegisterSchemeHandlerFactory`).
inline constexpr const char* kWebviewScheme = "cronymax-webview";

/// Strict default CSP applied to every served document. Extensions that
/// need to override (e.g. allow specific external hosts) embed their own
/// `<meta http-equiv="Content-Security-Policy">` tag — the more
/// restrictive of the two wins, so extensions can never *broaden* the
/// platform's baseline.
inline constexpr const char* kWebviewDefaultCsp =
    "default-src 'none'; "
    "script-src 'self' 'unsafe-inline'; "
    "style-src 'self' 'unsafe-inline'; "
    "img-src cronymax-webview: data:; "
    "connect-src 'self'; "
    "font-src 'self' data:";

/// Register the scheme name with CEF. Called from `App::
/// OnRegisterCustomSchemes` in **both** the browser and renderer process
/// app delegates — schemes must be declared identically on both sides or
/// the renderer treats the iframe URL as a non-standard "opaque" origin.
void RegisterWebviewScheme(CefRawPtr<CefSchemeRegistrar> registrar);

/// Install the scheme handler factory in the browser process. Call
/// from `App::OnContextInitialized` *after* `CefInitialize()` returns
/// (which it will by the time `OnContextInitialized` fires).
///
/// Pass the extensions root directory (`~/.cronymax/extensions`); each
/// request is resolved relative to that root.
void InstallWebviewSchemeHandlerFactory(const std::string& extensions_root);

}  // namespace cronymax
