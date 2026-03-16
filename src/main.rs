use log::info;
use std::sync::Arc;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    // Detect and set locale for i18n before any UI strings are accessed.
    cronymax::ui::i18n::detect_and_set_locale();

    info!("cronymax starting...");
    info!(
        "Platform: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    // Parse --profile <id> CLI argument (used by "New Window with Profile").
    let mut profile_override: Option<String> = None;
    {
        let args: Vec<String> = std::env::args().collect();
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--profile" && i + 1 < args.len() {
                profile_override = Some(args[i + 1].clone());
                i += 2;
            } else if let Some(value) = args[i].strip_prefix("--profile=") {
                profile_override = Some(value.to_string());
                i += 1;
            } else {
                i += 1;
            }
        }
    }
    if let Some(ref pid) = profile_override {
        info!("CLI: --profile override = '{}'", pid);
    }

    // Build a multi-threaded tokio runtime for async LLM/IO tasks.
    // winit owns the main thread; tokio tasks communicate back via EventLoopProxy.
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime"),
    );
    info!("Tokio runtime initialized");

    let config = cronymax::config::AppConfig::load();
    info!(
        "Configuration loaded: font='{}' @{}pt, shell={:?}",
        config.font.family, config.font.size, config.terminal.shell
    );

    cronymax::app::run(config, runtime, profile_override);
}
