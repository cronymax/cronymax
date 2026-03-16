use std::io::Write;
use std::time::Duration;

use cronymax::terminal::pty::Pty;

/// Helper: default shell for this platform.
fn default_shell() -> &'static str {
    if cfg!(target_os = "windows") {
        "cmd.exe"
    } else {
        "/bin/sh"
    }
}

#[test]
fn test_spawn_and_echo() {
    let mut pty = Pty::spawn(default_shell(), 80, 24, None);

    // Send a simple echo command and newline
    pty.writer.write_all(b"echo hello_pty_test\n").unwrap();
    pty.writer.flush().unwrap();

    // Wait a bit for the shell to process
    std::thread::sleep(Duration::from_millis(500));

    // Read whatever is available
    use std::io::Read;
    let mut buf = [0u8; 4096];
    // Set non-blocking-ish by reading with a timeout approach:
    // The reader might block, so use a thread with a timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut reader = pty.reader;
    std::thread::spawn(move || {
        let n = reader.read(&mut buf).unwrap_or(0);
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
    });

    let output = rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default();
    assert!(
        output.contains("hello_pty_test"),
        "Expected echo output, got: {:?}",
        output
    );
}

#[test]
fn test_pty_resize_no_panic() {
    let pty = Pty::spawn(default_shell(), 80, 24, None);
    // Resize should not panic.
    pty.resize(120, 40);
    pty.resize(40, 10);
    // Cleanup: kill child
    drop(pty);
}

#[test]
fn test_session_lifecycle() {
    use cronymax::terminal::TerminalSession;

    let mut session = TerminalSession::new(1, default_shell(), 80, 24, 1000, None, None);
    assert_eq!(session.id, 1);
    assert!(!session.exited);

    // Write to PTY
    session.write_to_pty(b"echo session_test\n");

    // Give shell time to process
    std::thread::sleep(Duration::from_millis(500));

    // Process output
    let processed = session.process_pty_output();
    assert!(processed, "Expected PTY output to be processed");

    // Resize
    session.resize(100, 30);
    assert_eq!(session.grid_size.cols, 100);
    assert_eq!(session.grid_size.rows, 30);
}

#[test]
fn test_session_exit_detection() {
    use cronymax::terminal::TerminalSession;

    let mut session = TerminalSession::new(2, default_shell(), 80, 24, 1000, None, None);

    // Tell the shell to exit
    session.write_to_pty(b"exit\n");

    // Wait for shell to exit
    std::thread::sleep(Duration::from_secs(1));

    // Drain output — eventually should detect exit
    for _ in 0..20 {
        session.process_pty_output();
        if session.exited {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(session.exited, "Session should detect shell exit");
}
