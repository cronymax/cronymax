use std::time::Duration;

use cronymax::renderer::terminal::pty::Pty;

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
    use std::io::{Read, Write};

    let mut pty = Pty::spawn(default_shell(), 80, 24, None);

    // On Windows ConPTY, cmd.exe may send a DSR cursor-position query
    // (\x1b[6n) and block until it gets a reply.  Send a minimal
    // response (\x1b[1;1R) to unblock the shell, then wait briefly
    // for the startup prompt before issuing the echo command.
    if cfg!(target_os = "windows") {
        pty.writer.write_all(b"\x1b[1;1R").unwrap();
        pty.writer.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    }

    pty.writer.write_all(b"echo hello_pty_test\n").unwrap();
    pty.writer.flush().unwrap();

    // Read in a background thread with accumulation and timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    let mut reader = pty.reader;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut accumulated = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    accumulated.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if accumulated.contains("hello_pty_test") {
                        break;
                    }
                }
                Err(_) => break,
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        let _ = tx.send(accumulated);
    });

    let output = rx.recv_timeout(Duration::from_secs(6)).unwrap_or_default();
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
    use cronymax::renderer::terminal::TerminalSession;

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
    use cronymax::renderer::terminal::TerminalSession;

    let mut session = TerminalSession::new(2, default_shell(), 80, 24, 1000, None, None);

    // On Windows ConPTY, cmd.exe may block on a DSR query until we send a
    // cursor-position response.  Send it through the pty_writer first.
    if cfg!(target_os = "windows") {
        session.write_to_pty(b"\x1b[1;1R");
    }

    // Give the shell time to initialize before sending exit.
    std::thread::sleep(Duration::from_millis(500));
    session.process_pty_output();

    // Tell the shell to exit
    session.write_to_pty(b"exit\r\n");

    // Wait for shell to exit — cmd.exe on Windows can take longer
    // than Unix shells, so use a generous timeout with frequent polling.
    for _ in 0..80 {
        session.process_pty_output();
        if session.exited {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(session.exited, "Session should detect shell exit");
}
